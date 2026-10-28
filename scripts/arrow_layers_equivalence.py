"""Equivalence of the canonical Arrow layers with ras2cng's Python pipeline.

Two steps, in two environments, so the reference never runs on the build under test:

    # 1. Reference: ras-commander, a released ras-commander-fast-hdf and ras2cng.
    python scripts/arrow_layers_equivalence.py oracle OUT FILE.gNN.hdf ... FILE.pNN.hdf ...
    # 2. Candidate: this build (feature python-arrow).
    python scripts/arrow_layers_equivalence.py compare OUT FILE.gNN.hdf ... FILE.pNN.hdf ...

`oracle` writes, per file and layer, an Arrow IPC file with the table ras2cng builds before its
GeoParquet writer: the geometry layers `merge_all_layers` extracts (`mesh_cells`, `mesh_faces`,
`mesh_areas`); each summary variable as a values table
(`get_mesh_summary_output(include_geometry=False)` with ras2cng's archive column types); and
ras2cng's polygon-mode result layer (cell values joined to the mesh-cell polygons of the plan's
geometry file, face values with the plan's face lines). `compare` reads the same layers through
`arrow_layer` (the alias `mesh_areas` included), rebuilds the polygon-mode layer with a plain
pyarrow join, and compares column names, Arrow types, rows, row order, values and WKB. Variables
ras-commander cannot read must be refused by the engine too.
"""
import json
import re
import sys
import time
from pathlib import Path

import pyarrow as pa
import pyarrow.compute as pc
import pyarrow.ipc as ipc


def _save(table, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    with pa.OSFile(str(path), "wb") as sink, ipc.new_file(sink, table.schema) as writer:
        writer.write_table(table)


def _read(path):
    with pa.memory_map(str(path)) as source:
        return ipc.open_file(source).read_all()


def _plan_geometry(plan):
    text = plan.with_suffix("").read_text(errors="replace")
    geom = re.search(r"^Geom File=(g\d+)", text, re.M).group(1)
    return plan.parent / f"{plan.name.split('.')[0]}.{geom}.hdf"


def oracle(out, files):
    import ras_commander_fast_hdf
    from ras2cng.geometry import _extract_hdf_layer, _prepare_for_parquet
    from ras2cng.geoparquet import _arrow_table, concat_layers
    from ras2cng.results import extract_results_variable, result_variable_slug, selected_summary_variables
    from ras_commander.hdf import HdfResultsMesh

    def plain(table):
        return table.cast(pa.schema(
            [pa.field(f.name, pa.string() if pa.types.is_large_string(f.type) else f.type) for f in table.schema]
        ))

    log = {"engine": ras_commander_fast_hdf.ENGINE_COMMIT, "files": {}}
    for hdf in files:
        record = log["files"][hdf.name] = {}
        if hdf.suffixes[-2].startswith(".g"):
            for layer in ("mesh_cells", "mesh_faces", "mesh_areas"):
                start = time.perf_counter()
                frame = _extract_hdf_layer(hdf, layer)
                if frame is None or len(frame) == 0:
                    record[layer] = {"rows": 0}
                    continue
                table = _arrow_table(concat_layers([_prepare_for_parquet(frame)]))
                record[layer] = {"rows": table.num_rows, "seconds": time.perf_counter() - start}
                _save(table, out / hdf.name / f"{layer}.arrow")
            continue
        geometry = _plan_geometry(hdf)
        cells = _extract_hdf_layer(geometry, "mesh_cells")
        record["_geometry"] = geometry.name
        for variable in selected_summary_variables(hdf):
            slug = result_variable_slug(variable)
            entry = record[slug] = {"variable": variable}
            try:
                start = time.perf_counter()
                frame = HdfResultsMesh.get_mesh_summary_output(hdf, variable, include_geometry=False)
                table = plain(pa.Table.from_pandas(concat_layers([frame], geometry=None), preserve_index=False))
                entry.update(rows=table.num_rows, seconds=time.perf_counter() - start)
                _save(table, out / hdf.name / f"values__{slug}.arrow")
                frame = extract_results_variable(hdf, variable, mesh_cells_gdf=cells, geometry_mode="polygon")
                table = _arrow_table(concat_layers([frame]))
                entry["pipeline_rows"] = table.num_rows
                _save(table, out / hdf.name / f"pipeline__{slug}.arrow")
            except Exception as exc:  # ras-commander and ras2cng skip such variables
                entry["error"] = f"{type(exc).__name__}: {exc}"
    (out / "oracle.json").write_text(json.dumps(log, indent=1))


def _difference(mine, ref):
    if mine.schema.names != ref.schema.names:
        return f"columns {mine.schema.names} != {ref.schema.names}"
    for a, b in zip(mine.schema, ref.schema):
        if a.type != b.type:
            return f"{a.name}: type {a.type} != {b.type}"
    if mine.num_rows != ref.num_rows:
        return f"rows {mine.num_rows} != {ref.num_rows}"
    for name in mine.schema.names:
        x, y = mine[name].combine_chunks(), ref[name].combine_chunks()
        if not x.equals(y):
            ne = pc.invert(pc.fill_null(pc.equal(x, y), False))
            return f"{name}: first difference at row {pc.index(ne, True).as_py()}"
    return None


def compare(out, files):
    import ras_commander_fast_hdf as native

    log = json.loads((out / "oracle.json").read_text())
    by_name = {f.name: f for f in files}

    def layer(path, name, variable=None):
        try:
            return pa.table(native.arrow_layer(str(path), name, variable)), None
        except Exception as exc:
            return None, f"refused: {type(exc).__name__}: {exc}"

    def bare(table):  # field metadata and nullability are framing, not data
        return pa.Table.from_arrays(table.columns, schema=pa.schema([pa.field(f.name, f.type) for f in table.schema]))

    rows, cells = [], {}
    for name, record in log["files"].items():
        path = by_name[name]
        if "_geometry" not in record:
            for layer_name, info in record.items():
                mine, why = layer(path, layer_name)
                if info["rows"] == 0:
                    status = "EQUAL (both empty)" if mine is not None and mine.num_rows == 0 else (why or "rows")
                else:
                    status = why or _difference(bare(mine), bare(_read(out / name / f"{layer_name}.arrow"))) or "EQUAL"
                rows.append((name, layer_name, info["rows"], status))
                if layer_name == "mesh_cells" and mine is not None:
                    cells[name] = mine
            continue
        geometry = record["_geometry"]
        polygons = cells.get(geometry) or layer(path.parent / geometry, "mesh_cells")[0]
        faces = layer(path, "mesh_faces")[0]
        for slug, info in record.items():
            if slug == "_geometry":
                continue
            mine, why = layer(path, "mesh_summary", info["variable"])
            if "error" in info:
                status = "EQUAL (both refuse)" if mine is None else f"oracle refuses, engine read {mine.num_rows} rows"
                rows.append((name, slug, 0, status))
                continue
            status = why or _difference(bare(mine), bare(_read(out / name / f"values__{slug}.arrow"))) or "EQUAL"
            rows.append((name, f"{slug} (values)", info["rows"], status))
            if mine is None:
                continue
            if "face_id" in mine.schema.names:
                v = mine.append_column("_row", pa.array(range(mine.num_rows), pa.int64()))
                joined = v.join(faces, ["mesh_name", "face_id"], join_type="left outer").sort_by("_row")
                joined = joined.select(mine.schema.names + ["geometry"])
            else:
                c = polygons.append_column("_row", pa.array(range(polygons.num_rows), pa.int64()))
                joined = c.join(mine, ["mesh_name", "cell_id"], join_type="left outer").sort_by("_row")
                keep = polygons.schema.names + [n for n in mine.schema.names if n not in ("mesh_name", "cell_id")]
                joined = joined.select(keep)
            status = _difference(bare(joined), bare(_read(out / name / f"pipeline__{slug}.arrow"))) or "EQUAL"
            rows.append((name, f"{slug} (ras2cng polygon layer)", info["pipeline_rows"], status))
    for row in rows:
        print(f"{row[0]:28s} {row[1]:56s} {row[2]:8d}  {row[3]}")
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
