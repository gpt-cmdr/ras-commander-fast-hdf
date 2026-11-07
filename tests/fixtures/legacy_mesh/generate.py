"""Boundary cases for the Names inventory and physical/ghost cell prefix.

Synthetic cases supplement the real HEC-RAS 5.0.x conformance test.
Run this script to regenerate fixtures and their SHA-256 manifest.
"""
import hashlib
import json
from pathlib import Path

import h5py
import numpy as np

HERE = Path(__file__).resolve().parent


def write(name, *, counts=(3, 4, 1, 1), gap=False, names=None, modern=False):
    path = HERE / (name + ".h5")
    with h5py.File(path, "w", libver="earliest") as f:
        g = f.require_group("Geometry/2D Flow Areas")
        g.create_dataset("Names", data=np.array(names if names is not None else [b"Zulu   ", b"Alpha  "], dtype="S16"))
        for area in ("Zulu", "Alpha"):
            mesh = g.create_group(area)
            mesh.attrs["Cell Maximum Index"] = np.int32(0)  # deliberately stale
            mesh.create_dataset("Cells Center Coordinate", data=np.zeros((len(counts), 2), dtype="<f8"))
            starts = np.r_[0, np.cumsum(counts[:-1])]
            if gap:
                starts[-1] += 1
            mesh.create_dataset("Cells Face and Orientation Info", data=np.column_stack([starts, counts]).astype("<i4"))
            mesh.create_dataset("Cells Face and Orientation Values", data=np.zeros((sum(counts), 2), dtype="<i4"))
        if modern:
            g.create_dataset("Attributes", data=np.array([(b"Alpha", 3), (b"Zulu", 3)], dtype=[("Name", "S16"), ("Cell Count", "<i4")]))
    return {"file": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


if __name__ == "__main__":
    files = [write("valid"), write("no_ghosts", counts=(3, 4)),
             write("two_faces", counts=(3, 2, 1, 1)),
             write("interleaved", counts=(3, 1, 4, 1)), write("gap", gap=True),
             write("duplicates", names=[b"Zulu", b"Zulu"]),
             write("missing_group", names=[b"Absent"]),
             write("invalid_name", names=[b"../Zulu"]),
             write("rank_two", names=[[b"Zulu"]]), write("modern", modern=True)]
    (HERE / "manifest.json").write_text(json.dumps({"generator": "generate.py", "files": files}, indent=2) + "\n")
