"""Write tests/fixtures/arrow_layers/arrow_features.h5, the synthetic geometry file used by
tests/arrow_layers.rs for the stored feature layers: boundary condition lines (a single-part
and a two-part line), storage areas (an open ring, a NaN attribute and a feature without
points), a two-part refinement region (refused) and pump stations (points).

    python tests/generate_arrow_features_fixture.py

Requires h5py and NumPy. The SHA-256 of the output is checked by the test.
"""
from pathlib import Path

import h5py
import numpy as np

OUT = Path(__file__).parent / "fixtures" / "arrow_layers" / "arrow_features.h5"


def packed(group, prefix, info, parts, points):
    group[f"{prefix} Info"] = np.array(info, dtype="<i4")
    group[f"{prefix} Parts"] = np.array(parts, dtype="<i4")
    group[f"{prefix} Points"] = np.array(points, dtype="<f8")


def main():
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with h5py.File(OUT, "w", libver="earliest", track_order=False) as h:
        h.attrs["File Type"] = np.bytes_(b"HEC-RAS Geometry")
        bc = h.create_group("Geometry/Boundary Condition Lines")
        bc["Attributes"] = np.array(
            [(b"Upstream", b"Area A", b"External"), (b"Split", b"Area A", b"Internal")],
            dtype=[("Name", "S16"), ("SA-2D", "S16"), ("Type", "S16")],
        )
        packed(
            bc,
            "Polyline",
            [[0, 2, 0, 1], [2, 5, 1, 2]],
            [[0, 2], [0, 2], [2, 3]],
            [[0, 0], [1, 0], [5, 5], [6, 5], [7, 7], [8, 7], [9, 8]],
        )
        sa = h.create_group("Geometry/Storage Areas")
        sa["Attributes"] = np.array(
            [(b"Pond", b"Elevation Volume", 12.5, np.nan, 3), (b"Empty", b"", 0.0, 1.0, 0)],
            dtype=[("Name", "S16"), ("Mode", "S24"), ("Avg Area", "<f4"), ("Min Elev", "<f4"),
                   ("Elev Vol Count", "<i4")],
        )
        # Row 1 has no points and one empty part, as HEC-RAS stores undrawn features.
        packed(sa, "Polygon", [[0, 3, 0, 1], [3, 0, 1, 1]], [[0, 3], [0, 0]],
               [[0, 0], [4, 0], [0, 3]])
        rr = h.create_group("Geometry/2D Flow Area Refinement Regions")
        rr["Attributes"] = np.array([(b"Two rings",)], dtype=[("Name", "S16")])
        packed(rr, "Polygon", [[0, 6, 0, 2]], [[0, 3], [3, 3]],
               [[0, 0], [9, 0], [0, 9], [1, 1], [2, 1], [1, 2]])
        ps = h.create_group("Geometry/Pump Stations")
        ps["Attributes"] = np.array(
            [(b"North PS", 3), (b"South PS", 1)], dtype=[("Name", "S16"), ("Pump Count", "<u2")]
        )
        ps["Points"] = np.array([[10.0, 20.0], [30.0, 40.0]], dtype="<f8")


if __name__ == "__main__":
    main()
