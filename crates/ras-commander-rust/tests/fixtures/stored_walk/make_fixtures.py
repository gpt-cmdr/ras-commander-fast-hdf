"""Synthetic HDF5 files for the stored-family walk tests (tests/stored_walk.rs).

Regenerate: python make_fixtures.py  (h5py 3.x). Python is not needed to run the Rust tests;
the manifest records each file's SHA-256.

- stored_walk.h5: a plan-like Output Blocks tree with a Base Output (2D, cross-section and
  reference-line series, breach variables) and two non-Base blocks holding numeric, string
  and compound datasets in nested groups, plus Plan Data / Event Conditions metadata.
- stored_walk_deep.h5: the same tree plus one branch nested 26 groups deep under a non-Base
  block, beyond the walk's depth limit of 24, so every walk of Output Blocks is refused.
"""
import hashlib
import json
from pathlib import Path

import h5py
import numpy as np

HERE = Path(__file__).resolve().parent
OUT = "Results/Unsteady/Output/Output Blocks"


def tree(f):
    rng = np.random.default_rng(20261024)
    f.attrs["File Type"] = np.bytes_(b"HEC-RAS Results")
    pd = f.create_group("Plan Data/Plan Information")
    pd.attrs["Plan Name"] = np.bytes_(b"Synthetic walk")
    f.create_dataset("Plan Data/Plan Parameters/Values", data=np.arange(6, dtype="f4"))
    f.create_dataset("Event Conditions/Unsteady/Boundary Conditions/Flow Hydrographs/Upstream",
                     data=rng.random((12, 2)).astype("f4"))
    base = f"{OUT}/Base Output/Unsteady Time Series"
    f.create_dataset(f"{base}/Time", data=np.arange(10, dtype="f8"))
    f.create_dataset(f"{base}/Time Date Stamp",
                     data=np.array([b"01JAN2000 %02d:00:00" % i for i in range(10)], dtype="S19"))
    for area in ("A1", "A2"):
        for var in ("Water Surface", "Depth", "Face Velocity"):
            f.create_dataset(f"{base}/2D Flow Areas/{area}/{var}", data=rng.random((10, 7)).astype("f4"))
    for var in ("Flow", "Water Surface"):
        f.create_dataset(f"{base}/Cross Sections/{var}", data=rng.random((10, 5)).astype("f4"))
    for var in ("Flow", "Water Surface"):
        f.create_dataset(f"{base}/Reference Lines/{var}", data=rng.random((10, 3)).astype("f4"))
    f.create_dataset(f"{base}/Inline Structures/Dam/Breaching Variables", data=rng.random((10, 4)).astype("f4"))
    rec = np.dtype([("Name", "S16"), ("Value", "<f8"), ("Count", "<i4")])
    for block in ("DSS Hydrograph Output", "Computation Block"):
        for g in range(6):
            for h in range(4):
                grp = f"{OUT}/{block}/Group {g}/Sub {h}"
                f.create_dataset(f"{grp}/Values", data=rng.random((8, 3)).astype("f8"))
                f.create_dataset(f"{grp}/Counts", data=rng.integers(0, 99, 9).astype("i4"))
                f.create_dataset(f"{grp}/Labels", data=np.array([b"lab%d" % i for i in range(4)], dtype="S8"))
                table = np.zeros(3, dtype=rec)
                table["Name"] = [b"a", b"b", b"c"]
                table["Value"] = rng.random(3)
                table["Count"] = [1, 2, 3]
                f.create_dataset(f"{grp}/Table", data=table)


def write(name, deep):
    path = HERE / name
    with h5py.File(path, "w", libver="earliest") as f:
        tree(f)
        if deep:
            grp = f"{OUT}/Computation Block/Deep" + "".join(f"/L{i:02d}" for i in range(26))
            f.create_dataset(f"{grp}/Bottom", data=np.arange(3, dtype="i4"))
    return {"file": name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


if __name__ == "__main__":
    manifest = {
        "generator": "make_fixtures.py (h5py, libver earliest); synthetic, no model data",
        "fixtures": [write("stored_walk.h5", False), write("stored_walk_deep.h5", True)],
    }
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, indent=2))
