"""Excerpts of HEC's public BaldEagleCrkMulti2D BaldEagleDamBrk.p07.hdf (HEC-RAS 5.0.3) for the
legacy row-label summary rule, plus mutations. Attributes are copied with their exact HDF5 types
(fixed-length strings, padding) by the low-level API. Usage:

    python generate_legacy_row_labels.py <BaldEagleDamBrk.p07.hdf>

Writes the .h5 files and manifest.json (SHA-256) next to this script.
"""
import hashlib
import json
import sys
from pathlib import Path

import h5py
import numpy as np

AREA = "BaldEagleCr"
SUMMARY = f"Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas/{AREA}"
GEOM = f"Geometry/2D Flow Areas/{AREA}"
N = 6  # cells kept
HERE = Path(__file__).resolve().parent


def copy_attrs(src, dst, rename=None, values=None, skip=()):
    for name in src.attrs:
        if name in skip:
            continue
        aid = h5py.h5a.open(src.id, name.encode())
        new_name = (rename or {}).get(name, name)
        value = (values or {}).get(name)
        tid = aid.get_type()
        data = np.empty(aid.shape, dtype=aid.dtype)
        aid.read(data)
        if value is not None:
            tid = tid.copy()
            tid.set_size(max(len(value), 1))
            data = np.array(value, dtype=f"S{max(len(value), 1)}")
        out = h5py.h5a.create(dst.id, new_name.encode(), tid, aid.get_space())
        out.write(data)


def write(name, src, *, variable="Maximum Water Surface", columns=N, rows=2, rename=None, values=None, extra=None, skip=()):
    path = HERE / name
    with h5py.File(path, "w", libver="earliest") as f:
        f.create_dataset(f"{GEOM}/Cells Center Coordinate", data=src[f"{GEOM}/Cells Center Coordinate"][:N])
        f.create_dataset(f"{GEOM}/Faces FacePoint Indexes", data=src[f"{GEOM}/Faces FacePoint Indexes"][: N + 1])
        s = src[f"{SUMMARY}/{variable}"]
        data = s[:rows, :columns] if rows <= 2 else np.vstack([s[:2, :columns], s[:1, :columns]])
        d = f.create_dataset(f"{SUMMARY}/{variable}", data=data)
        copy_attrs(s, d, rename, values, skip)
        for k, v in (extra or {}).items():
            d.attrs[k] = np.bytes_(v)
    return path


def main(source):
    with h5py.File(source, "r") as src:
        files = [
            write("valid.h5", src),
            write("units_meters.h5", src, values={"Row 0:  WSEL": "Meters"}),
            write("extra_attribute.h5", src, extra={"Units": "ft"}),
            write("wrong_count.h5", src, columns=N - 1),
            write("three_rows.h5", src, rows=3),
            write("face_velocity.h5", src, variable="Maximum Face Velocity", columns=N + 1),
            write("label_case.h5", src, rename={"Row 0:  WSEL": "Row 0: WSEL"}),
        ]
    manifest = {
        "source": "HEC BaldEagleCrkMulti2D example, BaldEagleDamBrk.p07.hdf (HEC-RAS 5.0.3)",
        "source_sha256": hashlib.sha256(Path(source).read_bytes()).hexdigest(),
        "generator": "generate_legacy_row_labels.py",
        "h5py": h5py.__version__,
        "files": [{"file": p.name, "bytes": p.stat().st_size, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for p in files],
    }
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main(sys.argv[1])
