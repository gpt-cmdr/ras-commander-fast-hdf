"""Read-only release benchmark driver; Python must be invoked with -I.

Fixtures are HEC-RAS plan HDF files with 2D results, supplied with
``--files PATH [PATH ...]`` or the RAS_PERF_FIXTURES environment variable.

One ABBA block per repetition: engine cold, h5py fresh, h5py reused,
engine reused. Raw vendored reader follows. Hashing is outside API timings.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time

import h5py
import numpy as np

# Fixture paths come from --files or from this environment variable
# (paths separated by os.pathsep: ';' on Windows, ':' elsewhere).
FILES_ENV = "RAS_PERF_FIXTURES"
BASE = "Results/Unsteady/Output/Output Blocks/Base Output"
GEOM = "Geometry/2D Flow Areas"


def discover(path):
    with h5py.File(path, "r") as f:
        areas = [(g["Cells Center Coordinate"].shape[0], name) for name, g in f[GEOM].items()
                 if isinstance(g, h5py.Group) and "Cells Center Coordinate" in g]
        n, area = max(areas)
        series = f"{BASE}/Unsteady Time Series/2D Flow Areas/{area}"
        summary = f"{BASE}/Summary Output/2D Flow Areas/{area}"
        water = f"{series}/Water Surface"
        nt, nc = f[water].shape
        summary_names = list(f[summary]) if summary in f else []
        maxname = next((x for x in ["Maximum Water Surface", "Maximum Water Surface Elevation"] if x in summary_names), None)
        if maxname is None:
            maxname = next((x for x in summary_names if "Maximum" in x and "Water" in x), None)
        rng = np.random.default_rng(20261019)
        sparse = rng.choice(nc, size=min(500, nc), replace=False).tolist()
        requests = []
        def add(workload, paths, rows, elements=None):
            requests.append(dict(file=str(path), area=area, workload=workload, paths=paths,
                                 dtypes=[f[p].dtype.str for p in paths], rows=rows, elements=elements))
        if maxname:
            p = f"{summary}/{maxname}"
            add("P1", [p], f[p].shape[0] if f[p].ndim == 2 else 1, [nc // 2])
        add("P2", [water], 1, sparse[:min(10, nc)])
        add("P3", [water], min(24, nt), sparse)
        add("P4", [water], min(24, nt), list(range(min(10000, nc))))
        add("B1", [water], nt)
        face = [f"{series}/{v}" for v in ["Face Velocity", "Face Flow"] if f"{series}/{v}" in f]
        if face:
            add("B2", face, nt)
        add("B3", [f"{GEOM}/{area}/{v}" for v in ["Cells Center Coordinate", "Faces Cell Indexes", "FacePoints Coordinate"]], 0)
        add("M1", [], 0)
        layouts = {p: dict(shape=f[p].shape, dtype=f[p].dtype.str, chunks=f[p].chunks,
                           compression=f[p].compression, compression_opts=f[p].compression_opts,
                           shuffle=f[p].shuffle, fletcher32=f[p].fletcher32,
                           stored_bytes=f[p].id.get_storage_size())
                   for q in requests for p in q["paths"]}
        return dict(file=str(path), size=Path(path).stat().st_size, area=area, stored_cells=n,
                    times=nt, result_cells=nc, layouts=layouts, requests=requests,
                    missing_workloads=[x for x in ["P1", "B2"] if x not in [q["workload"] for q in requests]])


def h5_call(f, q):
    if q["workload"] == "M1":
        # Explicitly a structural metadata baseline, not a semantic layer catalogue.
        found = []
        f.visititems(lambda n, o: found.append((n, o.shape, o.dtype.str)) if isinstance(o, h5py.Dataset) else None)
        return []
    out = []
    for p in q["paths"]:
        d = f[p]
        ids = q["elements"]
        if q["workload"] == "B3" or ids is None:
            out.append(d[...])
        elif ids == list(range(ids[0], ids[-1] + 1)):
            out.append(d[:q["rows"], ids[0]:ids[-1]+1] if d.ndim == 2 else d[ids[0]:ids[-1]+1])
        else:
            # HDF5 fancy indexing requires sorted indices. Restore caller order.
            order = np.argsort(ids)
            inverse = np.argsort(order)
            sorted_ids = np.asarray(ids)[order]
            a = d[:q["rows"], sorted_ids] if d.ndim == 2 else d[sorted_ids]
            out.append(a[:, inverse] if d.ndim == 2 else a[inverse])
    return out


def h5_measure(q, reused, fresh):
    t = time.perf_counter()
    f = h5py.File(q["file"], "r") if fresh else reused
    opened = (time.perf_counter()-t)*1000
    t = time.perf_counter()
    arrays = h5_call(f, q)
    call = (time.perf_counter()-t)*1000
    if fresh:
        f.close()
    return dict(mode="h5py_fresh" if fresh else "h5py_reused", open_ms=opened, call_ms=call,
                total_ms=opened+call, hashes=[hashlib.sha256(a.tobytes(order="C")).hexdigest() for a in arrays],
                bytes=[a.nbytes for a in arrays])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--exe", required=True)
    parser.add_argument("--reps", type=int, default=7)
    parser.add_argument("--discover-only", action="store_true")
    parser.add_argument("--files", nargs="+",
                        help=f"plan HDF files to probe (default: ${FILES_ENV}, os.pathsep-separated)")
    args = parser.parse_args()
    files = args.files or [p for p in os.environ.get(FILES_ENV, "").split(os.pathsep) if p]
    if not files:
        parser.error(f"no fixtures: pass --files or set {FILES_ENV}")
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    metadata = dict(start_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                    cpu_count=os.cpu_count(), python=platform.python_version(), h5py=h5py.__version__,
                    hdf5=h5py.version.hdf5_version, numpy=np.__version__,
                    free_disk={str(out.resolve()): shutil.disk_usage(out)._asdict()}, fixtures=[])
    for file in files:
        try:
            metadata["fixtures"].append(discover(file))
        except Exception as exc:
            metadata["fixtures"].append(dict(file=file, error=str(exc)))
    (out/"metadata.json").write_text(json.dumps(metadata, indent=2))
    if args.discover_only:
        return
    with (out/"rust-stderr.log").open("w") as err, (out/"samples.jsonl").open("w", buffering=1) as log:
        proc = subprocess.Popen([args.exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=err, text=True, bufsize=1)
        def rust(q, mode):
            proc.stdin.write(json.dumps(dict(q, mode=mode))+"\n")
            proc.stdin.flush()
            response = proc.stdout.readline()
            if not response:
                raise RuntimeError(f"Rust server exited: {proc.poll()}")
            return json.loads(response)
        try:
            for fixture in metadata["fixtures"]:
                if "error" in fixture:
                    continue
                with h5py.File(fixture["file"], "r") as reused:
                    for q in fixture["requests"]:
                        print(Path(q["file"]).name, q["workload"], flush=True)
                        for rep in range(args.reps):
                            # ABBA over API lifecycle modes; no OS-cache eviction.
                            for mode in ["cold", "h5py_fresh", "h5py_reused", "reused", "raw"]:
                                if mode == "raw" and q["workload"] == "M1":
                                    continue
                                t = time.perf_counter()
                                try:
                                    result = h5_measure(q, reused, mode == "h5py_fresh") if mode.startswith("h5py") else rust(q, mode)
                                except Exception as exc:
                                    result = dict(mode=mode, error=str(exc))
                                result.update(file=q["file"], workload=q["workload"], rep=rep,
                                              wall_ms=(time.perf_counter()-t)*1000)
                                log.write(json.dumps(result)+"\n")
        finally:
            proc.stdin.close()
            proc.wait(timeout=60)
    print("Measurement complete", flush=True)


if __name__ == "__main__":
    main()
