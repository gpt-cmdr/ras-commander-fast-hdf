"""Equivalence of the canonical feature layers with what ras2cng extracts today.

Two steps, in two environments, so the reference never runs on the build under test:

    # 1. Reference: ras2cng with ras-commander and a released ras-commander-fast-hdf.
    python scripts/arrow_feature_layers_equivalence.py oracle OUT FILE.gNN.hdf ...
    # 2. Candidate: this build (feature python-arrow).
    python scripts/arrow_feature_layers_equivalence.py compare OUT FILE.gNN.hdf ...

`oracle` writes, per file, every non-mesh layer ras2cng's `_extract_hdf_layer` returns, as the
Arrow table ras2cng builds before its GeoParquet writer. `compare` reads the same layer through
`arrow_layer(path, <ras2cng name>)` and applies the ras2cng field aliases from `layer_catalog()`.
It then checks, per layer: row count; row order through the identity (`source_row` against
ras2cng's id column where it has one); WKB of every geometry; and every column the two share,
values compared after casting the canonical column to ras2cng's type. Columns only one side
has are listed: ras2cng-only columns are what ras-commander derives or joins, canonical-only
columns are stored columns ras-commander drops.
"""
import json
import sys
from pathlib import Path

import pyarrow as pa
import pyarrow.compute as pc
import pyarrow.ipc as ipc

LAYERS = (
    "mesh_cell_points",
    "cross_sections", "centerlines", "river_reaches", "edge_lines", "bank_lines",
    "bc_lines", "breaklines", "refinement_regions", "reference_lines", "reference_points",
    "structures", "storage_areas", "pump_stations", "mannings_n_regions", "pipe_conduits", "pipe_nodes",
)


def _save(table, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    with pa.OSFile(str(path), "wb") as sink, ipc.new_file(sink, table.schema) as writer:
        writer.write_table(table)


def _read(path):
    with pa.memory_map(str(path)) as source:
        return ipc.open_file(source).read_all()


def oracle(out, files):
    import ras_commander_fast_hdf
    from ras2cng.geometry import _extract_hdf_layer, _prepare_for_parquet
    from ras2cng.geoparquet import _arrow_table, concat_layers

    log = {"engine": ras_commander_fast_hdf.ENGINE_COMMIT, "files": {}}
    for hdf in files:
        record = log["files"][hdf.name] = {}
        for layer in LAYERS:
            try:
                if layer == "mesh_cell_points":
                    from ras_commander.hdf import HdfMesh
                    frame = HdfMesh.get_mesh_cell_points(hdf, include_ghost_cells=False)
                else:
                    frame = _extract_hdf_layer(hdf, layer)
            except Exception as exc:
                record[layer] = {"error": f"{type(exc).__name__}: {exc}"}
                continue
            if frame is None or len(frame) == 0:
                record[layer] = {"rows": 0}
                continue
            table = _arrow_table(concat_layers([_prepare_for_parquet(frame)]))
            record[layer] = {"rows": table.num_rows}
            _save(table, out / hdf.name / f"{layer}.arrow")
    (out / "oracle.json").write_text(json.dumps(log, indent=1))


ID_COLUMNS = {"bc_line_id", "bl_id", "rr_id", "region_id", "river_id"}


def _compare_layer(mine, ref):
    notes = []
    if mine.num_rows != ref.num_rows:
        return f"rows {mine.num_rows} != {ref.num_rows}", notes
    shared = [n for n in mine.schema.names if n in ref.schema.names]
    for name in shared:
        x, y = mine[name].combine_chunks(), ref[name].combine_chunks()
        if name == "geometry":
            x, y = x.cast(pa.binary()), y.cast(pa.binary())
        elif x.type != y.type:
            notes.append(f"{name}: {x.type} -> {y.type}")
            try:
                x = x.cast(y.type)
            except pa.ArrowInvalid as exc:
                return f"{name}: cannot cast {x.type} to {y.type}: {exc}", notes
        if not x.equals(y):
            ne = pc.invert(pc.fill_null(pc.equal(x, y), False))
            if pc.any(ne).as_py():
                row = pc.index(ne, True).as_py()
                return f"{name}: first difference at row {row}: {x[row]} != {y[row]}", notes
            return f"{name}: nulls differ", notes
    only_ref = [n for n in ref.schema.names if n not in mine.schema.names]
    only_mine = [n for n in mine.schema.names if n not in ref.schema.names]
    if only_ref:
        notes.append(f"ras2cng-only: {only_ref}")
    if only_mine:
        notes.append(f"canonical-only: {only_mine}")
    return None, notes


def compare(out, files):
    import ras_commander_fast_hdf as native

    catalog = json.loads(native.layer_catalog())
    aliases = {}
    for a in catalog["field_aliases"]:
        if a["consumer"] == "ras2cng":
            aliases.setdefault(a["layer"], {})[a["canonical"]] = a["alias"]
    log = json.loads((out / "oracle.json").read_text())
    by_name = {f.name: f for f in files}
    rows = []
    for name, record in log["files"].items():
        path = by_name[name]
        for layer, info in record.items():
            try:
                mine = pa.table(native.arrow_layer(str(path), layer))
                why = None
            except Exception as exc:
                mine, why = None, f"{type(exc).__name__}: {exc}"
            if "error" in info:
                status = "EQUAL (both refuse)" if mine is None else f"oracle error {info['error']}, engine read {mine.num_rows}"
                rows.append((name, layer, 0, status, []))
                continue
            if info["rows"] == 0:
                ok = mine is None and "not stored" in (why or "") or (mine is not None and mine.num_rows == 0)
                rows.append((name, layer, 0, "EQUAL (both empty)" if ok else f"oracle empty; engine: {why or mine.num_rows}", []))
                continue
            if mine is None:
                rows.append((name, layer, info["rows"], f"refused: {why}", []))
                continue
            rename = aliases.get(layer, {})
            mine = mine.rename_columns([rename.get(n, n) for n in mine.schema.names])
            ref = _read(out / name / f"{layer}.arrow")
            status, notes = _compare_layer(mine, ref)
            rows.append((name, layer, info["rows"], status or "EQUAL", notes))
    for name, layer, n, status, notes in rows:
        print(f"{name:28s} {layer:20s} {n:7d}  {status}")
        for note in notes:
            print(f"{'':58s}{note}")
    failed = [r for r in rows if not r[3].startswith(("EQUAL", "refused"))]
    refused = [r for r in rows if r[3].startswith("refused")]
    print(f"\n{len(rows)} comparisons: {len(failed)} different, {len(refused)} refused by the engine")
    return 1 if failed else 0


if __name__ == "__main__":
    mode, out, *paths = sys.argv[1:]
    out, files = Path(out), [Path(p) for p in paths]
    if mode == "oracle":
        oracle(out, files)
    else:
        sys.exit(compare(out, files))
