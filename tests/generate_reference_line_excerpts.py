"""Reference-line geometry/result excerpts of a real plan, plus explicit identity mutations.

Usage: python -I generate_reference_line_excerpts.py --fixture Chippewa_2D.p02.hdf --output fixtures/reference_lines

Copies the stored `Geometry/Reference Lines` datasets, `Geometry/2D Flow Areas/Attributes`, and
the first 50 rows of the Base Output time axis and of the reference-line `Flow` and `Water
Surface` results, with the full `Name` axis (values and attributes unchanged), then writes one
file per mutation:

- valid: unchanged;
- reordered: result columns (and Name, Faces and 2D Areas) permuted to [2, 0, 1];
- bad_name: one Name entry matches no geometry row;
- duplicate_geometry: two geometry rows share Name and SA-2D;
- bad_faces: one `<name>|Faces` attribute differs from the geometry Internal Faces;
- bad_areas: one `2D Areas` ordinal names no area;
- no_internal_faces: geometry Internal Faces absent;
- no_geometry: Geometry/Reference Lines absent.
"""
from pathlib import Path
import argparse
import hashlib
import json

import h5py
import numpy as np

BASE = "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series"
LINES = BASE + "/Reference Lines"
GEOMETRY = "Geometry/Reference Lines"
ROWS = 50
MODES = ["valid", "reordered", "bad_name", "duplicate_geometry", "bad_faces", "bad_areas", "no_internal_faces", "no_geometry"]

p = argparse.ArgumentParser()
p.add_argument("--fixture", type=Path, required=True)
p.add_argument("--output", type=Path, required=True)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=True)
with h5py.File(a.fixture, "r") as src:
    geometry = [f"{GEOMETRY}/{k}" for k, v in src[GEOMETRY].items() if isinstance(v, h5py.Dataset)]
    paths = geometry + ["Geometry/2D Flow Areas/Attributes", BASE + "/Time", BASE + "/Time Date Stamp"]
    paths += [f"{LINES}/{v}" for v in ["Name", "Flow", "Water Surface"]]
    order = [2, 0, 1]
    for mode in MODES:
        with h5py.File(a.output / f"{mode}.h5", "w", libver="earliest") as dst:
            for path in paths:
                if mode == "no_geometry" and path.startswith(GEOMETRY + "/"):
                    continue
                if mode == "no_internal_faces" and path == f"{GEOMETRY}/Internal Faces":
                    continue
                x = src[path]
                data = x[()]
                if path.startswith(BASE + "/") and not path.endswith("/Name"):
                    data = data[:ROWS]  # the same leading rows of Time, stamps and results
                if mode == "reordered" and path.startswith(LINES + "/"):
                    data = data[order] if path.endswith("/Name") else data[:, order]
                if mode == "bad_name" and path == f"{LINES}/Name":
                    data = data.copy()
                    data[0] = b"not_a_line|" + data[0].split(b"|")[1]
                if mode == "duplicate_geometry" and path == f"{GEOMETRY}/Attributes":
                    data = data.copy()
                    data["Name"][2] = data["Name"][0]
                y = dst.create_dataset(path, data=data, dtype=x.dtype, compression="gzip")
                for k, v in x.attrs.items():
                    if mode == "reordered" and path.startswith(LINES + "/") and k == "2D Areas":
                        v = v[order]
                    if mode == "bad_faces" and path == f"{LINES}/Name" and k.endswith("|Faces"):
                        v = v.copy()
                        v[0] = v[0] + 1
                    if mode == "bad_areas" and k == "2D Areas":
                        v = v.copy()
                        v[0] = 5
                    y.attrs[k] = v
(a.output / "provenance.json").write_text(json.dumps(dict(
    source_name=a.fixture.name,
    source_sha256=hashlib.sha256(a.fixture.read_bytes()).hexdigest(),
    source="HEC public example 2D Sediment Transport/Chippewa_2D, plan p02 computed with HEC-RAS 7.0",
    mutations="see the module docstring of generate_reference_line_excerpts.py",
    files={f.name: hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted(a.output.glob("*.h5"))},
), indent=2))
