"""Per-area reference-line excerpts of a HEC-RAS 7.0.1 plan, plus explicit identity mutations.

Usage: python -I generate_per_area_reference_line_excerpts.py --fixture Weise.p01.hdf --output fixtures/reference_lines

HEC-RAS 7.0.1 also writes each area's reference-line series under
`Unsteady Time Series/2D Flow Areas/<area>/Reference Lines`, with bare `<name>` entries and no
Faces attributes. Copies the stored `Geometry/Reference Lines` datasets,
`Geometry/2D Flow Areas/Attributes`, the Base Output time axis and the block and per-area
reference-line `Name` and `Flow` datasets (values and attributes unchanged), then writes one file
per mutation:

- per_area_valid: unchanged;
- per_area_reordered: per-area result columns (and Name) permuted to [2, 0, 1];
- per_area_bad_name: one per-area Name entry matches no geometry row of that area;
- per_area_no_block_name: the block's `Reference Lines` group (with the Faces attributes) absent;
- per_area_bad_faces: one block `<name>|Faces` attribute differs from the geometry Internal Faces.
"""
from pathlib import Path
import argparse
import hashlib
import json

import h5py

BASE = "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series"
LINES = BASE + "/Reference Lines"
GEOMETRY = "Geometry/Reference Lines"
MODES = ["per_area_valid", "per_area_reordered", "per_area_bad_name", "per_area_no_block_name", "per_area_bad_faces"]

p = argparse.ArgumentParser()
p.add_argument("--fixture", type=Path, required=True)
p.add_argument("--output", type=Path, required=True)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=True)
with h5py.File(a.fixture, "r") as src:
    (area,) = list(src[BASE + "/2D Flow Areas"])
    per_area = f"{BASE}/2D Flow Areas/{area}/Reference Lines"
    geometry = [f"{GEOMETRY}/{k}" for k, v in src[GEOMETRY].items() if isinstance(v, h5py.Dataset)]
    paths = geometry + ["Geometry/2D Flow Areas/Attributes", BASE + "/Time", BASE + "/Time Date Stamp"]
    paths += [f"{LINES}/{v}" for v in ["Name", "Flow"]] + [f"{per_area}/{v}" for v in ["Name", "Flow"]]
    order = [2, 0, 1] + list(range(3, len(src[per_area + "/Name"])))
    written = {}
    for mode in MODES:
        out = a.output / f"{mode}.h5"
        with h5py.File(out, "w", libver="earliest") as dst:
            for path in paths:
                if mode == "per_area_no_block_name" and path.startswith(LINES + "/"):
                    continue
                x = src[path]
                data = x[()]
                if mode == "per_area_reordered" and path.startswith(per_area + "/"):
                    data = data[order] if path.endswith("/Name") else data[:, order]
                if mode == "per_area_bad_name" and path == f"{per_area}/Name":
                    data = data.copy()
                    data[0] = b"not_a_line"
                y = dst.create_dataset(path, data=data, dtype=x.dtype, compression="gzip")
                for k, v in x.attrs.items():
                    if mode == "per_area_bad_faces" and path == f"{LINES}/Name" and k.endswith("|Faces"):
                        v = v.copy()
                        v[0] = v[0] + 1
                    y.attrs[k] = v
            dst.attrs["File Version"] = src.attrs["File Version"]
        written[out.name] = hashlib.sha256(out.read_bytes()).hexdigest()
prov = a.output / "provenance_per_area.json"
prov.write_text(json.dumps(dict(
    source_name=a.fixture.name,
    source_sha256=hashlib.sha256(a.fixture.read_bytes()).hexdigest(),
    source="HEC public example 2D Sediment Transport/Weise_2D, plan p01 (sediment off), reference lines "
           "and points added with ras-commander GeomReferenceFeatures and computed with HEC-RAS 7.0.1 by CLB "
           "(corpus entry hec701r/Weise.p01.reflines)",
    mutations="see the module docstring of generate_per_area_reference_line_excerpts.py",
    files=written,
), indent=2) + "\n")
