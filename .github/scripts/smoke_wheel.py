"""Smoke test an installed ras-commander-fast-hdf wheel.

Usage: python -I smoke_wheel.py <dist-dir> <expected-commit>
"""
import glob
import sys
import zipfile

import ras_commander_fast_hdf as native

dist, commit = sys.argv[1], sys.argv[2]
assert native.API_VERSION == 11, native.API_VERSION
assert native.ENGINE_COMMIT == commit, (native.ENGINE_COMMIT, commit)
assert tuple(native.ENGINE_FEATURES) == ("catalog", "hdf", "mesh", "results"), native.ENGINE_FEATURES
for name in ("read_numeric", "HdfSession", "assemble_cells", "write_pmtiles",
             "export_ipc_file", "export_pmtiles_ipc_file"):
    assert hasattr(native, name), name
# Release wheels are built without the test-hooks feature.
assert not hasattr(native, "_panic_for_test"), "release wheel exposes _panic_for_test"
assert not hasattr(native.HdfSession, "_panic_for_test"), "release wheel exposes HdfSession._panic_for_test"

wheel = zipfile.ZipFile(glob.glob(f"{dist}/*.whl")[0])
licences = {n.rsplit("/", 1)[-1] for n in wheel.namelist() if ".dist-info/licenses/" in n}
missing = {"THIRD-PARTY-NOTICES.md", "ras-commander-LICENSE.txt", "ras-commander-fast-hdf-LICENSE.txt"} - licences
assert not missing, f"wheel licence files missing: {missing}"
print(sys.version.split()[0], native.__version__, native.API_VERSION, native.ENGINE_COMMIT)
