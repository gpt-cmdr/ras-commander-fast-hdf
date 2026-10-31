"""Write tests/fixtures/arrow_layers/arrow_layers.h5, the synthetic plan used by
tests/arrow_layers.rs: one 2D flow area with two physical cells and one ghost cell,
a curved face, a stored perimeter, and three summary variables in HEC-RAS 6.x layout.

    python tests/generate_arrow_layers_fixture.py

Requires h5py and NumPy. The SHA-256 of the output is checked by the test.
"""
from pathlib import Path

import h5py
import numpy as np

OUT = Path(__file__).parent / "fixtures" / "arrow_layers" / "arrow_layers.h5"
BASE = "Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas/Area A"


def s16(values):
    return np.array([v.encode() for v in values], dtype="S16")


def main():
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with h5py.File(OUT, "w", libver="earliest", track_order=False) as h:
        h.attrs["File Type"] = np.bytes_(b"HEC-RAS Results")
        attrs = np.array([(b"Area A", 2)], dtype=[("Name", "S16"), ("Cell Count", "<i4")])
        h.create_dataset("Geometry/2D Flow Areas/Attributes", data=attrs)
        g = h.create_group("Geometry/2D Flow Areas/Area A")
        points = np.array([[0, 0], [1, 0], [2, 0], [0, 1], [1, 1], [2, 1]], dtype="<f8")
        faces = np.array([[0, 1], [1, 2], [2, 5], [5, 4], [4, 3], [3, 0], [1, 4]], dtype="<i4")
        perim_info = np.zeros((7, 2), dtype="<i4")
        perim_info[1] = [0, 1]  # face 1 bends through (1.5, -0.25)
        perim_values = np.array([[1.5, -0.25]], dtype="<f8")
        cell_info = np.array([[0, 4], [4, 4], [8, 1]], dtype="<i4")
        cell_values = np.array(
            [[0, 1], [6, 1], [4, 1], [5, 1], [1, 1], [2, 1], [3, 1], [6, -1], [0, -1]], dtype="<i4"
        )
        g["FacePoints Coordinate"] = points
        g["Faces FacePoint Indexes"] = faces
        g["Faces Perimeter Info"] = perim_info
        g["Faces Perimeter Values"] = perim_values
        g["Cells Face and Orientation Info"] = cell_info
        g["Cells Face and Orientation Values"] = cell_values
        g["Cells Center Coordinate"] = np.array([[0.5, 0.5], [1.5, 0.5], [0.5, -0.5]], dtype="<f8")
        g["Perimeter"] = np.array([[0, 0], [2, 0], [2, 1], [0, 1]], dtype="<f4")
        info = h.create_group("Plan Data/Plan Information")
        info.attrs["Simulation Start Time"] = np.bytes_(b"01JAN2020 00:00:00")
        wse = h.create_dataset(
            f"{BASE}/Maximum Water Surface",
            data=np.array([[10.0, np.nan, 99.0], [0.5, 1.25, 2.0]], dtype="<f4"),
        )
        wse.attrs["Rows Variables"] = s16(["WSEL", "Time"])
        wse.attrs["Units"] = s16(["ft", "days"])
        vel = h.create_dataset(
            f"{BASE}/Maximum Face Velocity",
            data=np.array([[1, 2, 3, 4, 5, 6, 7], [1, 2, 1, 0, 3, 1, 2]], dtype="<f4"),
        )
        vel.attrs["Location"] = np.bytes_(b"Faces")
        vel.attrs["Rows Variables"] = s16(["Velocity", "Time"])
        vel.attrs["Units"] = np.bytes_(b"ft/s")
        it = h.create_dataset(f"{BASE}/Cell Cumulative Iteration", data=np.array([0, 3, 9], dtype="<f4"))
        it.attrs["Location"] = np.bytes_(b"Cells")
        it.attrs["Units"] = np.bytes_(b"Iterations")
        bad = h.create_dataset(f"{BASE}/Starting Differences WSE", data=np.zeros((3, 3), dtype="<f4"))
        bad.attrs["Location"] = np.bytes_(b"Cells")


if __name__ == "__main__":
    main()
