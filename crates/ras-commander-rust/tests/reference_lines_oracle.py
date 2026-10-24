"""Independent h5py oracle for `reference_lines_match_real_h5py_links`.

Usage: python -I reference_lines_oracle.py <plans-dir> <out.json> <plan.hdf>...

For each plan and each reference-line Unsteady Time Series dataset with one row per Time row and
one column per Name entry, writes the `Geometry/Reference Lines/Attributes` row of every column
and the SHA-256 of the stored values (little-endian). A column's row is the unique geometry row
whose `Name|SA-2D` equals the stored Name entry, and whose `Internal Faces` equal the Name axis's
`<Name>|Faces` attribute; anything else stops the oracle.
"""
import hashlib
import json
import sys
from pathlib import Path

import h5py
import numpy as np

SERIES = "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series"
LINES = SERIES + "/Reference Lines"


def text(v):
    return bytes(v).decode("utf-8").rstrip("\x00").rstrip()


def main(root, out, files):
    report = {}
    for rel in files:
        path = Path(root) / rel
        with h5py.File(path, "r") as f:
            g = f["Geometry/Reference Lines"]
            attrs = g["Attributes"][()]
            keys = [f"{text(a['Name'])}|{text(a['SA-2D'])}" for a in attrs]
            internal = g["Internal Faces"][()]
            name_ds = f[f"{LINES}/Name"]
            rows = []
            for entry in (text(n) for n in name_ds[()]):
                hits = [i for i, k in enumerate(keys) if k == entry]
                assert len(hits) == 1, (rel, entry, hits)
                row = hits[0]
                faces = internal["Face Index"][internal["Reference Line ID"] == row]
                stored = name_ds.attrs[f"{text(attrs[row]['Name'])}|Faces"]
                assert list(faces) == list(stored), (rel, entry)
                rows.append(row)
            time_rows = len(f[f"{SERIES}/Time"])
            series = {}
            for name, d in f[LINES].items():
                if name == "Name" or not isinstance(d, h5py.Dataset) or d.shape != (time_rows, len(rows)):
                    continue
                v = d[()]
                series[name] = {"rows": rows, "values_sha256": hashlib.sha256(v.astype(v.dtype.newbyteorder("<")).tobytes()).hexdigest()}
            report[rel] = {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "series": series}
    Path(out).write_text(json.dumps(report, indent=1))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3:])
