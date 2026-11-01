"""Check the installed wheel on invented schemas and pinned public HDF files."""

import argparse
import hashlib
import json
from pathlib import Path

import h5py
import ras_commander_fast_hdf as native


def equal(actual, expected):
    assert actual.shape == expected.shape
    assert actual.dtype == expected.dtype
    assert actual.tobytes() == expected.tobytes()


def check_points(path):
    value = native.initial_condition_point_geometry(str(path))["value"]
    assert value is not None
    with h5py.File(path) as hdf:
        equal(value["points"], hdf["Geometry/IC Points/Points"][()])
        attrs = hdf["Geometry/IC Points/Attributes"][()]
        assert len(value["features"]) == len(attrs)
        for row, feature in enumerate(value["features"]):
            assert feature["identity"]["row"] == row
            assert feature["identity"]["table"] == "Geometry/IC Points/Attributes"
            assert feature["name"] == attrs["Name"][row].decode().rstrip("\x00 ")
            assert feature["area"] == attrs["SA/2D"][row].decode().rstrip("\x00 ")
            assert feature["cell_index"] == int(attrs["Cell Index"][row])


def check_blocks(path):
    value = native.read_xs_geometry(str(path), "blocked_obstructions")
    with h5py.File(path) as hdf:
        equal(value["info"], hdf["Geometry/Cross Sections/Obstruction Info"][()])
        expected = hdf["Geometry/Cross Sections/Obstruction Blocks"][()]
        assert value["values_kind"] == "compound"
        assert tuple(value["values"]["shape"]) == expected.shape
        assert value["values"]["raw_bytes"] == expected.tobytes()


def check_steady(path):
    base = "Results/Steady/Output/Output Blocks/Base Output/Steady Profiles"
    with h5py.File(path) as hdf:
        rows = len(hdf[base + "/Profile Names"])
        for name in ["Velocity Total", "Area including Ineffective Total"]:
            relative = "Additional Variables/" + name
            value = native.read_xs_result(str(path), "steady", relative, (0, rows, 1))
            equal(value["values"], hdf[base + "/Cross Sections/" + relative][()])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path)
    args = parser.parse_args()
    path = Path(__file__).parent / "fixtures/data_readers/valid.h5"
    check_points(path)
    check_blocks(path)
    check_steady(path)
    with h5py.File(path) as hdf:
        dataset = "Event Conditions/Observed Data/Flow/Ref Line: Gauge"
        value = native.read_family_dataset(str(path), "event_conditions", dataset)
        assert value["storage"] == "compound"
        assert value["values"]["raw_bytes"] == hdf[dataset][()].tobytes()
    if args.corpus:
        raw = (args.corpus / "manifest.json").read_bytes()
        assert (
            hashlib.sha256(raw).hexdigest()
            == "f3be980317bf514babf4fd2f3a15986898a38a6d90657f869e55c909517f622a"
        )
        entries = {e["id"]: e for e in json.loads(raw)["entries"]}
        for identity, checks in [
            ("rbfs/DowntownCorpusChris.g12", [check_points]),
            ("hec701r/Baxter.p01", [check_blocks]),
            ("hec66/Baxter.p01", [check_steady]),
        ]:
            entry = entries[identity]
            path = args.corpus / entry["path"]
            assert path.stat().st_size == entry["bytes"]
            assert hashlib.sha256(path.read_bytes()).hexdigest() == entry["sha256"]
            for check in checks:
                check(path)
    print("Stored data wheel checks passed; corpus=" + str(args.corpus is not None))


if __name__ == "__main__":
    main()
