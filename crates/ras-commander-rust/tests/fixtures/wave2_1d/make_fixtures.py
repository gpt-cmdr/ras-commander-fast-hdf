"""Generate the small synthetic 1D (API 7) fixtures and their SHA256 manifest.

Usage: python -I make_fixtures.py   (writes next to this file; requires h5py + numpy only)
Each file isolates one absent / unsupported / corrupt layout; real-file semantics are covered
by the h5py oracle (oracle_wave2_1d.py) instead.
"""
import hashlib
import json
from pathlib import Path

import h5py
import numpy as np

HERE = Path(__file__).resolve().parent
XS_DT = np.dtype([("River", "S16"), ("Reach", "S16"), ("RS", "S8"), ("Name", "S16"), ("Left Bank", "<f4"), ("Right Bank", "<f4")])
TS = "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series"
STRUCT_DT = np.dtype([("Type", "S16"), ("River", "S16"), ("Reach", "S16"), ("RS", "S8"), ("Connection", "S16")])


def xs_table(g):
    g.create_dataset("Attributes", data=np.array([(b"R", b"A", b"2", b"", 1, 2), (b"R", b"A", b"1", b"", 1, 2)], dtype=XS_DT))


def xs_fields(path):
    with h5py.File(path, "w") as f:
        g = f.create_group("Geometry/Cross Sections")
        xs_table(g)
        # Station-elevation: second range runs past the Values array -> Corrupt.
        g["Station Elevation Info"] = np.array([[0, 2], [2, 5]], dtype="<i4")
        g["Station Elevation Values"] = np.zeros((4, 2), dtype="<f4")
        # Manning's n: Info without Values -> Corrupt (incomplete pair).
        g["Manning's n Info"] = np.array([[0, 1], [1, 1]], dtype="<i4")
        # Deck High: floating Info -> Unsupported(encoding).
        g["Deck High Info"] = np.array([[0, 1], [1, 1]], dtype="<f4")
        g["Deck High Values"] = np.zeros((2, 2), dtype="<f4")
        # Deck Low: valid pair; exact f64 values and an unsorted/repeated selection.
        g["Deck Low Info"] = np.array([[0, 2], [2, 1]], dtype="<i4")
        g["Deck Low Values"] = np.array([[1.5, 2.5], [3.5, -0.0], [np.nan, 7.25]], dtype="<f8")
        g["Orthogonal Vectors"] = np.zeros((2, 2), dtype="<f4")


def xs_legacy(path):
    with h5py.File(path, "w") as f:
        g = f.create_group("Geometry/Cross Sections")
        xs_table(g)
        g["Node Names"] = np.array([b"", b""], dtype="S16")


def results(path):
    with h5py.File(path, "w") as f:
        xs_table(f.create_group("Geometry/Cross Sections"))
        g = f.create_group(TS + "/Cross Sections")
        g["Cross Section Attributes"] = np.array([(b"R", b"A", b"2", b""), (b"R", b"A", b"1", b"")], dtype=[("River", "S16"), ("Reach", "S16"), ("Station", "S8"), ("Name", "S16")])
        f[TS + "/Time"] = np.array([0.0, 0.5, 1.0])
        g["Water Surface"] = np.arange(6, dtype="<f4").reshape(3, 2)
        g["Velocity Total"] = np.zeros((4, 2), dtype="<f4")  # rows disagree with Time -> Corrupt
        g["Velocity Channel"] = np.zeros((3, 5), dtype="<f4")  # no section axis -> Unsupported
        # Rank 3 with the right shape but no Dimension attribute: orientation unproven.
        g["Components"] = np.zeros((3, 4, 2), dtype="<f4")
        # Steady rank 3 with Dimension: qualified; 2 profiles x 3 components x 2 sections.
        sp = "Results/Steady/Output/Output Blocks/Base Output/Steady Profiles"
        f[sp + "/Profile Names"] = np.array([b"P1", b"P2"], dtype="S16")
        ident = g["Cross Section Attributes"][()]
        f["Results/Steady/Output/Geometry Info/Cross Section Attributes"] = ident
        cv = np.arange(12, dtype="<f4").reshape(2, 3, 2)
        d = f.create_dataset(sp + "/Cross Sections/Cross Section Variables", data=cv)
        d.attrs["Dimension"] = np.array([b"Profiles", b"Variables", b"Cross Sections"], dtype="S32")
        s = f.create_group("Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/Cross Sections")
        d = s.create_dataset("Maximum Flow", data=np.arange(6, dtype="<f4").reshape(3, 2))
        d.attrs["Variable_Unit"] = np.array([[b"Max Flow", b"cfs"], [b"WSEL at Max Flow", b"ft"], [b"Time at Max Flow", b"days"]], dtype="S32")
        s["Bad Rank"] = np.zeros((2, 2, 2), dtype="<f4")
        # A structure family below Summary Output: unqualified layout -> Unsupported(schema).
        u = f.create_group("Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/Inline Structures")
        u["Maximum Flow"] = np.zeros((3, 1), dtype="<f4")


def structures(path):
    with h5py.File(path, "w") as f:
        f["Geometry/Structures/Attributes"] = np.array([(b"Inline", b"R", b"A", b"5", b""), (b"Bridge", b"R", b"A", b"7", b"")], dtype=STRUCT_DT)
        f["Geometry/Structures/Profile Data"] = np.zeros((40, 2), dtype="<f4")
        f[TS + "/Time"] = np.array([0.0, 1.0])
        g = f.create_group(TS + "/Inline Structures/R A 9")  # no matching geometry row
        g["Structure Variables"] = np.zeros((2, 2), dtype="<f4")
        b = f.create_group(TS + "/Bridges")
        b["Stray"] = np.zeros((2, 1), dtype="<f4")  # dataset where a group is required -> Corrupt
        c = f.create_group(TS + "/Culverts/R A 7")  # identity row exists but Type is Bridge
        c["Culvert Variables"] = np.zeros((2, 1), dtype="<f4")
        # Steady bridge results: matching identity and rows, but unqualified layout.
        sp = "Results/Steady/Output/Output Blocks/Base Output/Steady Profiles"
        f[sp + "/Profile Names"] = np.array([b"P1", b"P2"], dtype="S16")
        sb = f.create_group(sp + "/Bridges/R A 7")
        sb["Bridge Variables"] = np.zeros((2, 3), dtype="<f4")


def bad_connection(path):
    """Matching inline identity, but the Connection column is an integer, not text."""
    dt = np.dtype([("Type", "S16"), ("River", "S16"), ("Reach", "S16"), ("RS", "S8"), ("Connection", "<i4")])
    with h5py.File(path, "w") as f:
        f["Geometry/Structures/Attributes"] = np.array([(b"Inline", b"R", b"A", b"5", 7)], dtype=dt)
        f[TS + "/Time"] = np.array([0.0, 1.0])
        g = f.create_group(TS + "/Inline Structures/R A 5")
        g["Structure Variables"] = np.zeros((2, 2), dtype="<f4")


def lines(path):
    """Multi-part packed lines (Parts relative to each feature) and bad ranges."""
    with h5py.File(path, "w") as f:
        g = f.create_group("Geometry/River Bank Lines")
        g["Polyline Info"] = np.array([[0, 5, 0, 2], [5, 2, 2, 1]], dtype="<i4")
        g["Polyline Parts"] = np.array([[0, 3], [3, 2], [0, 2]], dtype="<i4")
        g["Polyline Points"] = np.arange(14, dtype="<f8").reshape(7, 2)
        e = f.create_group("Geometry/River Edge Lines")
        e["Polyline Info"] = np.array([[0, 2, 0, 1]], dtype="<i4")
        e["Polyline Parts"] = np.array([[1, 2]], dtype="<i4")  # part runs past its feature
        e["Polyline Points"] = np.zeros((2, 2), dtype="<f8")
        t = f.create_group("Geometry/Cross Section Interpolation Surfaces")
        t["TIN Info"] = np.array([[0, 3, 0, 2]], dtype="<i4")  # two triangles, one stored
        t["TIN Points"] = np.zeros((3, 4), dtype="<f8")
        t["TIN Triangles"] = np.array([[0, 1, 2]], dtype="<i4")


def main():
    makers = {"lines.h5": lines, "bad_connection.h5": bad_connection, "xs_fields.h5": xs_fields, "xs_legacy.h5": xs_legacy, "results.h5": results, "structures.h5": structures}
    manifest = {"generator": "make_fixtures.py", "h5py": h5py.__version__, "numpy": np.__version__, "fixtures": {}}
    for name, make in makers.items():
        p = HERE / name
        make(p)
        manifest["fixtures"][name] = hashlib.sha256(p.read_bytes()).hexdigest()
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")


if __name__ == "__main__":
    main()
