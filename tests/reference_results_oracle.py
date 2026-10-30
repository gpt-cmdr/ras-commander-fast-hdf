"""h5py-only oracle for the reference result joins (`reference_line_results`,
`reference_point_results`).

For each plan HDF in a directory, the oracle joins every numeric reference series to its
geometry row through the stored Name axis alone: global series store `<name>|<area>`,
per-area series (`.../2D Flow Areas/<area>/Reference Lines|Points/...`) store `<name>` and
take the area from the path. It records, per geometry row, the name, a digest of the row's
stored coordinates and the (dataset, column, digest of the column's stored bytes) pairs.
Series whose Name entries do not each match exactly one row are listed as unjoinable.

Usage: python tests/reference_results_oracle.py --fixture-dir DIR --output oracle.json
"""
import argparse
import hashlib
import json
from pathlib import Path

import h5py
import numpy as np

BLOCKS = "Results/Unsteady/Output/Output Blocks"


def sha(a):
    return hashlib.sha256(np.ascontiguousarray(a).tobytes()).hexdigest()


def text(values):
    return [v.decode("utf-8").rstrip("\x00 ") if isinstance(v, bytes) else str(v) for v in values]


def geometry(h, kind):
    if kind == "lines":
        g = h.get("Geometry/Reference Lines")
        if g is None:
            return None
        a = g["Attributes"][()]
        names, areas = text(a["Name"]), text(a["SA-2D"])
        info, points = g["Polyline Info"][()], g["Polyline Points"][()]
        coords = [points[int(i[0]): int(i[0]) + int(i[1])] for i in info]
    else:
        g = h.get("Geometry/Reference Points")
        if g is None:
            return None
        a = g["Attributes"][()]
        names, areas = text(a["Name"]), text(a["SA/2D"])
        points = g["Points"][()]
        coords = [points[i: i + 1] for i in range(len(names))]
    return [{"name": n, "area": ar, "geometry_sha256": sha(c), "series": []}
            for n, ar, c in zip(names, areas, coords)]


def series_groups(h, group_name):
    """(group path, area or None) for every reference series group of every output block."""
    out = []
    if BLOCKS not in h:
        return out
    for block in h[BLOCKS]:
        ts = f"{BLOCKS}/{block}/Unsteady Time Series"
        if ts not in h:
            continue
        if f"{ts}/{group_name}" in h:
            out.append((f"{ts}/{group_name}", None))
        if f"{ts}/2D Flow Areas" in h:
            for area in h[f"{ts}/2D Flow Areas"]:
                p = f"{ts}/2D Flow Areas/{area}/{group_name}"
                if p in h:
                    out.append((p, area))
    return out


def join(h, kind):
    rows = geometry(h, kind)
    if rows is None:
        return None, []
    group_name = "Reference Lines" if kind == "lines" else "Reference Points"
    unjoinable = []
    for group, area in series_groups(h, group_name):
        g = h[group]
        if "Name" not in g:
            continue
        names = text(g["Name"][()])
        targets = []
        for entry in names:
            hits = [i for i, r in enumerate(rows)
                    if (area is None and f"{r['name']}|{r['area']}" == entry)
                    or (area is not None and r["name"] == entry and r["area"] == area)]
            targets.append(hits[0] if len(hits) == 1 else None)
        for var, d in g.items():
            if var == "Name" or not isinstance(d, h5py.Dataset) or d.ndim != 2 \
                    or d.dtype.kind != "f":
                continue
            path = f"{group}/{var}"
            if None in targets or d.shape[1] != len(names):
                unjoinable.append(path)
                continue
            values = d[()]
            for column, row in enumerate(targets):
                rows[row]["series"].append([path, column, sha(values[:, column])])
    return rows, unjoinable


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--fixture-dir", required=True)
    p.add_argument("--output", required=True)
    a = p.parse_args()
    out = {}
    for f in sorted(Path(a.fixture_dir).glob("*.hdf")):
        with h5py.File(f, "r") as h:
            entry = {"sha256": hashlib.sha256(f.read_bytes()).hexdigest()}
            for kind in ("lines", "points"):
                rows, unjoinable = join(h, kind)
                entry[kind] = {"rows": rows, "unjoinable": unjoinable}
        out[f.name] = entry
    Path(a.output).write_text(json.dumps(out, indent=1))
    print(f"{len(out)} plans")


if __name__ == "__main__":
    main()
