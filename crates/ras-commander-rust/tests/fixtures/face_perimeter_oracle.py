"""Reference digests for the opt-in face-line/perimeter test, from ras-commander's Python readers.

Calls ras-commander's `HdfMesh.get_mesh_cell_faces` and `HdfMesh.get_mesh_areas` with the native
engine turned off (RAS_COMMANDER_FAST_HDF=h5py), so the reference is the h5py/shapely path, and
writes per file and mesh: the face count, a SHA-256 over every face line (vertex count, then x, y
as little-endian f64), and the same digest for the perimeter ring (None when the area stores no
`Perimeter` dataset). Usage:

    python face_perimeter_oracle.py <ras-commander checkout> <hdf folder> <out.json> <file>...
"""
import hashlib
import json
import os
import struct
import sys
from pathlib import Path

os.environ["RAS_COMMANDER_FAST_HDF"] = "h5py"


def digest(lines):
    h = hashlib.sha256()
    for coords in lines:
        h.update(struct.pack("<Q", len(coords)))
        for x, y in coords:
            h.update(struct.pack("<dd", x, y))
    return h.hexdigest()


def main(checkout, root, out, files):
    sys.path.insert(0, checkout)
    import h5py
    from ras_commander.hdf.HdfMesh import HdfMesh

    report = {}
    for name in files:
        path = Path(root) / name
        faces = HdfMesh.get_mesh_cell_faces(path)
        areas = HdfMesh.get_mesh_areas(path)
        if areas.empty:
            # No reference: ras-commander reads no 2D areas here (for example HEC-RAS 5.0 files
            # without Geometry/2D Flow Areas/Attributes, or 1D-only files).
            print(f"skipped {name}: ras-commander returns no 2D areas", file=sys.stderr)
            continue
        with h5py.File(path, "r") as f:
            stored = {m: f"Geometry/2D Flow Areas/{m}/Perimeter" in f for m in areas["mesh_name"]}
        entry = {}
        for mesh in areas["mesh_name"]:
            rows = faces[faces["mesh_name"] == mesh].sort_values("face_id")
            assert list(rows["face_id"]) == list(range(len(rows)))
            lines = [[(float(x), float(y)) for x, y, *_ in g.coords] for g in rows.geometry]
            ring = areas[areas["mesh_name"] == mesh].geometry.iloc[0].exterior.coords
            entry[mesh] = {
                "faces": len(rows),
                "faces_sha256": digest(lines),
                "perimeter_sha256": digest([[(float(x), float(y)) for x, y, *_ in ring]]) if stored[mesh] else None,
            }
        report[name] = entry
    Path(out).write_text(json.dumps(report, indent=2))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:])
