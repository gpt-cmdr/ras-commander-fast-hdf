"""Generate the Python reference for the ring-order oracle (tests/ring_order_oracle.rs).

The reference is ras-commander's own Python output (the ring-order contract, option a):
`HdfMesh.get_mesh_cell_polygons` (GEOS polygonize path, no native backend) plus
`HdfMesh.diagnose_mesh_cell_polygons`. Run with PYTHONPATH at a ras-commander tree whose
`get_mesh_cell_polygons` takes no `backend` argument or is forced to the Python path, and with
shapely, geopandas, h5py and numpy installed:

    PYTHONPATH=<ras-commander main clone> python generate_ring_order_reference.py \
        fixtures.txt OUT_DIR

`fixtures.txt` holds `name|absolute path` lines. Fixtures are opened read-only and hashed
(sha256). Per fixture and mesh the script writes raw little-endian arrays: `*.coords.f64`
(rings concatenated, (N, 2)), `*.offsets.u64` (ring start offsets, rings + 1 entries) and
`*.cells.i64` (source cell ID per ring), plus `manifest.json`.
"""
import hashlib
import json
import os
import sys
from pathlib import Path

import numpy as np
import shapely

os.environ.setdefault("RAS_COMMANDER_FAST_HDF", "0")
from ras_commander import HdfMesh  # noqa: E402


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 22), b""):
            h.update(block)
    return h.hexdigest()


def main():
    fixtures, out = Path(sys.argv[1]), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    manifest = {"shapely": shapely.__version__, "geos": shapely.geos_version_string, "entries": []}
    for line in fixtures.read_text().splitlines():
        if not line.strip():
            continue
        name, path = line.split("|")
        gdf = HdfMesh.get_mesh_cell_polygons(Path(path))
        diag = HdfMesh.diagnose_mesh_cell_polygons(Path(path))
        for mesh in sorted(set(gdf.mesh_name.astype(str))):
            sub = gdf[gdf.mesh_name == mesh]
            geoms = sub.geometry.to_numpy()
            assert all(g.geom_type == "Polygon" for g in geoms)
            _, coords, (ring_off, poly_off) = shapely.to_ragged_array(geoms)
            assert np.all(np.diff(poly_off) == 1), "unexpected holes"
            stem = f"{name}.{mesh}".replace("/", "_").replace(" ", "_")
            np.ascontiguousarray(coords[:, :2], dtype="<f8").tofile(out / f"{stem}.coords.f64")
            np.ascontiguousarray(ring_off, dtype="<u8").tofile(out / f"{stem}.offsets.u64")
            np.ascontiguousarray(sub.cell_id.to_numpy(), dtype="<i8").tofile(out / f"{stem}.cells.i64")
            d = diag[diag.mesh_name.astype(str) == mesh]
            manifest["entries"].append(
                {
                    "fixture": name,
                    "path": path.replace("\\", "/"),
                    "sha256": sha256(path),
                    "mesh": mesh,
                    "stem": stem,
                    "rings": int(len(sub)),
                    "coordinates": int(len(coords)),
                    "diagnostics": [
                        {
                            "cell_id": int(r.cell_id),
                            "face_count": int(r.face_count),
                            "polygon_count": int(r.polygon_count),
                            "status": str(r.status),
                            "reason_code": str(r.reason_code),
                        }
                        for r in d.itertuples()
                    ],
                }
            )
            print(name, mesh, len(sub), flush=True)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1))


main()
