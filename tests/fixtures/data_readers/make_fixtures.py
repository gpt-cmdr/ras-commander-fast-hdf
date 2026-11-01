"""Generate invented stored-data schemas and record their checksums."""

import hashlib
import json
from pathlib import Path

import h5py
import numpy as np

root = Path(__file__).parent
with h5py.File(root / "valid.h5", "w") as h:
    h["Geometry/IC Points/Attributes"] = np.array(
        [(b"Initial", b"Area", 0), (b"Initial", b"Area", 1)],
        dtype=[("Name", "S16"), ("SA/2D", "S16"), ("Cell Index", "<i4")],
    )
    h["Geometry/IC Points/Points"] = np.array([[0.0, -0.0], [1.0, 2.0]], dtype="<f8")
    h["Geometry/2D Flow Areas/Area/Cells Center Coordinate"] = np.zeros((2, 2))
    h["Geometry/Cross Sections/Attributes"] = np.array(
        [
            (b"River", b"Reach", b"2", b"", 0.0, 1.0),
            (b"River", b"Reach", b"1", b"", 0.0, 1.0),
        ],
        dtype=[
            ("River", "S16"),
            ("Reach", "S16"),
            ("RS", "S8"),
            ("Name", "S16"),
            ("Left Bank", "<f4"),
            ("Right Bank", "<f4"),
        ],
    )
    h["Geometry/Cross Sections/Obstruction Info"] = np.array(
        [[0, 1], [1, 1]], dtype="<i4"
    )
    h["Geometry/Cross Sections/Obstruction Blocks"] = np.array(
        [(0.0, 1.0, -0.0), (2.0, 3.0, np.nan)],
        dtype=[("Left Sta", "<f4"), ("Right Sta", "<f4"), ("Elevation", "<f4")],
    )
    d = h.create_dataset(
        "Event Conditions/Observed Data/Flow/Ref Line: Gauge",
        data=np.array(
            [(b"01JAN2020 2400", 0.0, -0.0), (b"01JAN2020 2400", 0.0, np.nan)],
            dtype=[("Date", "S18"), ("Simulation Time", "<f8"), ("Value", "<f4")],
        ),
    )
    d.attrs["Data Units"] = np.bytes_("cfs")
    sp = "Results/Steady/Output/Output Blocks/Base Output/Steady Profiles"
    h[sp + "/Profile Names"] = np.array([b"P1", b"P2"], dtype="S16")
    h["Results/Steady/Output/Geometry Info/Cross Section Attributes"] = np.array(
        [(b"River", b"Reach", b"2", b""), (b"River", b"Reach", b"1", b"")],
        dtype=[("River", "S16"), ("Reach", "S16"), ("Station", "S8"), ("Name", "S16")],
    )
    for name in ["Velocity Total", "Area including Ineffective Total"]:
        h[sp + "/Cross Sections/Additional Variables/" + name] = np.array(
            [[1.0, -0.0], [np.nan, 4.0]], dtype="<f4"
        )
with h5py.File(root / "partial.h5", "w") as h:
    h["Geometry/IC Points/Attributes"] = np.array(
        [], dtype=[("Name", "S1"), ("SA/2D", "S1"), ("Cell Index", "<i4")]
    )
manifest = {
    p.name: hashlib.sha256(p.read_bytes()).hexdigest()
    for p in sorted(root.glob("*.h5"))
}
(root / "SHA256.json").write_text(json.dumps(manifest, indent=2) + "\n")
