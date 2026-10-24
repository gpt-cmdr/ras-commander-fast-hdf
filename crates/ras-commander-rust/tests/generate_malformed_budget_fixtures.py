"""Generate the engine budget fixtures in tests/fixtures/malformed/ (h5py).

Each file is tiny but declares a huge chunked dataset with no written chunks,
so a whole read would allocate gigabytes of fill values. The read budgets
must refuse them before allocating.

    python tests/generate_malformed_budget_fixtures.py <output-dir>
"""
import os
import sys

import h5py

out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "fixtures", "malformed")


def make(name, build):
    path = os.path.join(out, name)
    with h5py.File(path, "w", libver="latest") as f:
        build(f)
    print(name, os.path.getsize(path))


# 2^28 x 2 f64 = 4 GiB declared mesh coordinates: over the 2 GiB default budget.
make("budget_mesh_coords_4GiB.h5", lambda f: f.create_dataset(
    "Geometry/2D Flow Areas/Area/FacePoints Coordinate", shape=(1 << 28, 2),
    dtype="<f8", chunks=(1024, 2), fillvalue=0.0))
# 2^25 f64 = 256 MiB declared: under the default, over a small explicit budget.
make("budget_numeric_256MiB.h5", lambda f: f.create_dataset(
    "data", shape=(1 << 25,), dtype="<f8", chunks=(4096,), fillvalue=0.0))
