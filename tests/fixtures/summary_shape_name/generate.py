"""Synthetic Summary Output arrays without a stored `Location`, for the absent-Location
summary rule (`DomainRule::SummaryShapeAndName`, `tests/summary_shape_name.rs`).

No model data: every value is made up. The attribute layouts are the ones HEC-RAS writes on
such summaries, as fixed-length space-padded text:

- HEC-RAS 6.1: `Rows` = [quantity, `Time`] and `Units` = [unit, `days`] (plus `Max/Min Value`,
  `Max/Min Time` scalars on some);
- HEC-RAS 5.0.x: only `Row 0:  <quantity>` = unit and `Row 1:  Time` = `Days`;
- rank-one iteration counts with a single text attribute `Cell`.

`valid.h5`: mesh `A` with 4 stored cells (3 physical, `Cell Count` 3, one ghost) and 6 faces.
`equal_counts.h5`: mesh `A` with 4 cells and 4 faces, so the width cannot tell the domain.

Usage: python -I generate.py   (writes the .h5 files and manifest.json next to this script)
"""
import hashlib
import json
from pathlib import Path

import h5py
import numpy as np

HERE = Path(__file__).resolve().parent
SUMMARY = "Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas/A"


def text_attr(obj, name, values, size=16):
    """A fixed-length, space-padded text attribute (scalar for a str, 1-D for a list)."""
    tid = h5py.h5t.C_S1.copy()
    tid.set_size(size)
    tid.set_strpad(h5py.h5t.STR_SPACEPAD)
    scalar = isinstance(values, str)
    items = [values] if scalar else list(values)
    data = np.array([v.encode().ljust(size) for v in items], dtype=f"S{size}")
    space = h5py.h5s.create(h5py.h5s.SCALAR) if scalar else h5py.h5s.create_simple((len(items),))
    aid = h5py.h5a.create(obj.id, name.encode(), tid, space)
    aid.write(data.reshape(()) if scalar else data, mtype=tid)


def base(f, cells, faces, physical):
    f.attrs["File Type"] = np.bytes_(b"HEC-RAS Results")
    f.attrs["File Version"] = np.bytes_(b"HEC-RAS 6.1.0 September 2021")
    f.create_dataset(
        "Geometry/2D Flow Areas/Attributes",
        data=np.array([(b"A", physical)], dtype=[("Name", "S16"), ("Cell Count", "<i4")]),
    )
    g = f.create_group("Geometry/2D Flow Areas/A")
    g.create_dataset("Cells Center Coordinate", data=np.arange(cells * 2, dtype="<f8").reshape(cells, 2))
    g.create_dataset("Faces FacePoint Indexes", data=np.zeros((faces, 2), dtype="<i4"))
    g.create_dataset("FacePoints Coordinate", data=np.zeros((faces + 1, 2), dtype="<f8"))
    info = f.create_group("Plan Data/Plan Information")
    info.attrs["Simulation Start Time"] = np.bytes_(b"01JAN2020 00:00:00")
    return f.create_group(SUMMARY)


def pair(n, seed):
    values = (np.arange(n, dtype="<f4") * 1.5 + seed).astype("<f4")
    days = (np.arange(n, dtype="<f4") / 8 + 0.25).astype("<f4")
    return np.vstack([values, days])


def write_valid(path):
    with h5py.File(path, "w") as f:
        s = base(f, cells=4, faces=6, physical=3)
        d = s.create_dataset("Maximum Face Velocity", data=pair(6, 1.0))
        text_attr(d, "Rows", ["Velocity", "Time"])
        text_attr(d, "Units", ["ft/s", "days"])
        d.attrs["Max Value"] = np.float32(8.5)
        d = s.create_dataset("Maximum Cell Courant", data=pair(4, 0.1))
        text_attr(d, "Rows", ["Courant", "Time"])
        text_attr(d, "Units", ["outflow*dt/vol", "days"])
        d = s.create_dataset("Maximum Water Surface", data=pair(4, 100.0))
        text_attr(d, "Rows", ["WSEL", "Time"])
        text_attr(d, "Units", ["ft", "days"])
        d = s.create_dataset("Maximum Face Shear Stress", data=pair(6, 0.5))
        text_attr(d, "Row 0:  Shear Stress", "lbf/ft^2")
        text_attr(d, "Row 1:  Time", "Days", size=4)
        d = s.create_dataset("Cell Last Iteration", data=np.array([0, 3, 1, 7], dtype="<i4"))
        text_attr(d, "Cell", "Last cell to converge plus maximum iterations", size=45)
        # Refused: not an element array, more than two rows, row 1 not Time, a width the
        # name contradicts, a width matching nothing.
        d = s.create_dataset("Cell Cumulative Iter Lookup", data=np.zeros((1, 2), dtype="<i4"))
        text_attr(d, "Column 1", "Cell Number")
        d = s.create_dataset("Starting Differences WSE", data=np.zeros((3, 4), dtype="<f4"))
        text_attr(d, "Row0", "Prior Profile WSE", size=24)
        d = s.create_dataset("Maximum Face Courant", data=pair(6, 2.0))
        text_attr(d, "Rows", ["Courant", "Count"])
        text_attr(d, "Units", ["vel*dt/length", "days"])
        d = s.create_dataset("Minimum Face Velocity", data=pair(4, 3.0))
        text_attr(d, "Rows", ["Velocity", "Time"])
        text_attr(d, "Units", ["ft/s", "days"])
        d = s.create_dataset("Cell Volume Error", data=np.zeros(5, dtype="<f4"))


def write_equal(path):
    with h5py.File(path, "w") as f:
        s = base(f, cells=4, faces=4, physical=3)
        d = s.create_dataset("Maximum Face Velocity", data=pair(4, 1.0))
        text_attr(d, "Rows", ["Velocity", "Time"])
        text_attr(d, "Units", ["ft/s", "days"])
        d = s.create_dataset("Cell Last Iteration", data=np.array([0, 3, 1, 7], dtype="<i4"))
        text_attr(d, "Cell", "Last cell to converge plus maximum iterations", size=45)


def main():
    files = [HERE / "valid.h5", HERE / "equal_counts.h5"]
    write_valid(files[0])
    write_equal(files[1])
    manifest = {
        "generator": "generate.py",
        "files": [{"file": p.name, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for p in files],
    }
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
