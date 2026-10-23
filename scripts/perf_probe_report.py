"""Summarize perf_probe JSONL without discarding errors or hash differences."""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import numpy as np


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--inputs", nargs="+", required=True)
    p.add_argument("--out", required=True)
    args = p.parse_args()
    out = Path(args.out)
    fixtures, samples = [], []
    for directory in args.inputs:
        root = Path(directory)
        fixtures.extend(json.loads((root/"metadata.json").read_text())["fixtures"])
        samples.extend(json.loads(line) for line in (root/"samples.jsonl").read_text().splitlines())
    grouped = defaultdict(list)
    verification = []
    for s in samples:
        grouped[s["file"], s["workload"], s["mode"]].append(s)
    summary = []
    def metric(rows, key):
        vals = [r[key] for r in rows if key in r and r[key] is not None and "error" not in r]
        return dict(n=len(vals), median=float(np.median(vals)), q25=float(np.percentile(vals,25)),
                    q75=float(np.percentile(vals,75)), iqr=float(np.percentile(vals,75)-np.percentile(vals,25))) if vals else None
    def fmt(rows, key="total_ms"):
        stats = metric(rows, key)
        return f"{stats['median']:.3f} ({stats['iqr']:.3f})" if stats else "N/A"
    lines = ["# Release engine performance probe", "", "All timing tables use milliseconds: median (IQR = Q75 − Q25), seven observations per mode. Source arrays are read-only.", "",
             "## Method and interpretation", "",
             "Release profile uses thin LTO, one codegen unit; default engine session has one Rayon worker and parallel reads enabled. Python runs with -I. Each repetition follows ABBA: engine fresh session, h5py fresh File, h5py reused File, engine reused session; direct vendored reader follows. Sessions/Files persist across the fixture's workloads. There is no untimed warm-up, so repetition zero includes first-session costs. Fresh/cold means fresh API session, not OS-cache cold. File close/destruction and verification hashing are excluded; owned result allocation/copy is included.", "",
             "Direct reader uses the same dataset paths, row bounds and unsorted elements, and the RAS reader's adjacent-request chunk coalescing. It calls serial read_slice while the engine calls read_slice_parallel in its one-worker pool. Consequently engine-minus-raw includes validation, repeated dataset/type proof and thread dispatch as well as RAS metadata overhead; it is an attribution bound, not an isolated causal subtraction. Raw serialization into canonical bytes is measured then subtracted from its call time. Engine/h5py serialization and SHA256 are outside API timers.", "",
             "Raw Storage wraps the existing FileStorage with Instant/atomic counters. Its I/O time includes all storage ranges during the call (metadata plus chunks), including FileStorage buffer allocation and OS reads, not only payload reads or wire time. Raw lookup is dataset-handle lookup, not chunk-index traversal. Chunk index, filters/decompression, cache handling and numeric decoding remain combined in read_slice; these cannot be split through the unmodified public API. The requested fallback of cold/reused/domain-vs-direct reads is therefore used. Inventory diagnostic runs after each engine call and is warm metadata timing; it is not subtracted as an exact in-call stage.", "",
             "P1 requests one middle cell from Maximum Water Surface. The domain API returns both stored summary rows (value and time); every comparator reads the same two-row column. A single value-row API is unavailable, so P1 is a two-value point-query proxy. P2 uses one row and ten deterministic unsorted cells. P3 uses up to 24 rows and 500 distinct unsorted cells (seed 20261019). P4 uses up to 24 rows and the first min(10000, cells) contiguous cells. B1/B2 read all rows and cells/faces. B2 uses Face Velocity; Face Flow is absent in these files. B3 reads three complete exact geometry arrays. M1 engine is layer_inventory; h5py is recursive dataset metadata enumeration and is only a structural baseline, not equivalent semantic output. Raw M1 is N/A; metadata bytes are not value-hashed.", "",
             "h5py sparse reads sort indices for its required fancy-index order and restore caller order with NumPy. This preserves exact requested values but uses HDF5's own query strategy. Reused h5py retains File, not Dataset handles; dataset chunk-cache retention may differ from the Rust session-global cache. Both default cache settings are left unchanged. Concurrent handles can share HDF5 internal file metadata. ABBA reduces drift but does not equalize caches or remove workload-order effects.", ""]
    for fixture in fixtures:
        file = fixture["file"]
        lines.extend([f"## {Path(file).name}", "", f"Source: `{file}`", ""])
        if "error" in fixture:
            lines.extend([f"Discovery error: {fixture['error']}", ""])
            continue
        lines.extend([f"Largest area: **{fixture['area']}**; {fixture['result_cells']:,} stored result cells; {fixture['times']:,} timesteps; {fixture['size']:,} file bytes.", "",
                      "| Workload | Engine cold | Engine reused | Raw reader reused | h5py fresh | h5py reused | Hash |", "|---|---:|---:|---:|---:|---:|---|"])
        for q in fixture["requests"]:
            workload = q["workload"]
            bymode = {mode:grouped[file, workload, mode] for mode in ["cold", "reused", "raw", "h5py_fresh", "h5py_reused"]}
            hashes = {tuple(s["hashes"]) for rows in bymode.values() for s in rows if "hashes" in s}
            errors = [s for rows in bymode.values() for s in rows if "error" in s]
            okay = workload != "M1" and len(hashes) == 1 and not errors and all(len(rows) == 7 for rows in bymode.values())
            status = "MATCH" if okay else "N/A metadata" if workload == "M1" else "FAIL/INCOMPLETE"
            verification.append(dict(file=file, workload=workload, status=status, distinct_hashes=[list(h) for h in hashes], errors=errors))
            lines.append("| " + " | ".join([workload]+[fmt(rows) for rows in bymode.values()]+[status]) + " |")
            summary.append(dict(file=file, workload=workload, modes={mode:{key:metric(rows,key) for key in ["total_ms","open_ms","call_ms","inventory_diagnostic_ms","lookup_ms","read_combined_ms","gather_ms"]} for mode,rows in bymode.items()}, verification=status))
        lines.extend(["", "### Measured stages", "", "I/O is a subset of raw lookup + combined read, so these columns must not be added together. Unlisted time covers allocation, bookkeeping and object lifetimes. Inventory diagnostic is a separate warm probe.", "",
                      "| Workload | Cold open | Reused inventory diagnostic | Raw lookup | Raw combined read | Raw sparse scatter | Raw storage I/O | Raw cache misses/hits |", "|---|---:|---:|---:|---:|---:|---:|---:|"])
        for q in fixture["requests"]:
            w = q["workload"]
            raw = grouped[file,w,"raw"]
            io = [dict(io_ms=s["cache"]["io_ms"]) for s in raw if isinstance(s.get("cache"),dict)]
            counts = [(s["cache"]["misses"],s["cache"]["hits"]) for s in raw if isinstance(s.get("cache"),dict)]
            counttext = f"{np.median([x[0] for x in counts]):.0f}/{np.median([x[1] for x in counts]):.0f}" if counts else "N/A"
            lines.append("| " + " | ".join([w,fmt(grouped[file,w,"cold"],"open_ms"),fmt(grouped[file,w,"reused"],"inventory_diagnostic_ms"),fmt(raw,"lookup_ms"),fmt(raw,"read_combined_ms"),fmt(raw,"gather_ms"),fmt(io,"io_ms"),counttext]) + " |")
        lines.extend(["", "### Chunk layouts and filters", "", "| Dataset | Shape | Chunks | Dtype | Compression | Shuffle |", "|---|---|---|---|---|---|"])
        for path, layout in fixture["layouts"].items():
            lines.append(f"| {path.rsplit('/',1)[-1]} | {layout['shape']} | {layout['chunks']} | {layout['dtype']} | {layout['compression']} {layout['compression_opts']} | {layout['shuffle']} |")
        lines.append("")
    (out/"summary.json").write_text(json.dumps(summary,indent=2))
    (out/"verification.json").write_text(json.dumps(verification,indent=2))
    (out/"perf-tables.md").write_text("\n".join(lines),encoding="utf-8")
    print("Rows",len(summary),"verification",{x:sum(v['status']==x for v in verification) for x in sorted({v['status'] for v in verification})})


if __name__ == "__main__":
    main()
