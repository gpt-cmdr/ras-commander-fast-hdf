"""Independent QA of the ring-order contract (h5py + numpy + shapely only, no ras_commander).

    python verify_ring_contract.py REFERENCE_DIR NATIVE_DIR REPORT.json

REFERENCE_DIR is written by generate_ring_order_reference.py (Python / GEOS polygonize output)
and NATIVE_DIR by the ignored Rust test `ring_order_oracle` (RAS_RING_ORDER_NATIVE_DUMP). For every
fixture and mesh this script

  (a) recomputes every ring from the raw HDF arrays with a closed-form numpy/pure-Python rule
      (clockwise; start at the first listed face traversed in the cell's clockwise direction;
      consecutive duplicates dropped) and requires equality, bit for bit, with BOTH backends;
  (b) requires shapely.is_ccw to be False for every ring of every source;
  (c) requires identical cell sets in both backends, including the omitted cells and their
      diagnostics, and the omitted set to equal the numpy reference's (cells with fewer than three
      faces) plus the cells whose reference ring shapely finds invalid;
  (d) requires the shapely WKB of every polygon to be equal between the two backends.

Exit status is nonzero on any mismatch. Fixtures are opened read-only.
"""
import collections
import json
import sys
from pathlib import Path

import h5py
import numpy as np
import shapely


def load(directory, stem):
    d = Path(directory)
    coords = np.fromfile(d / f"{stem}.coords.f64", dtype="<f8").reshape(-1, 2)
    offsets = np.fromfile(d / f"{stem}.offsets.u64", dtype="<u8").astype(np.int64)
    cells = np.fromfile(d / f"{stem}.cells.i64", dtype="<i8")
    return coords, offsets, cells


def polygons(coords, offsets):
    counts = np.diff(offsets)
    rings = shapely.linearrings(coords, indices=np.repeat(np.arange(len(counts)), counts))
    return rings, shapely.polygons(rings)


def reference_ring(g, cell):
    """Closed-form contract ring for one cell, from the raw HDF arrays."""
    start, count = (int(v) for v in g["cinfo"][cell][:2])
    rows = g["cvals"][start : start + count]
    faces = [int(r[0]) for r in rows]
    ends, pinfo, pv, pc = g["fpi"], g["pinfo"], g["pv"], g["fpc"]
    adjacency = collections.defaultdict(list)
    for i, f in enumerate(faces):
        adjacency[int(ends[f][0])].append(i)
        adjacency[int(ends[f][1])].append(i)
    f0 = faces[0]
    # orientation +1: the cell is left of A->B, so clockwise runs B->A from B.
    first = int(ends[f0][1]) if int(rows[0][1]) == 1 else int(ends[f0][0])
    point = first
    visited = [False] * len(faces)
    ring = [tuple(pc[first])]
    for _ in faces:
        i = next(i for i in adjacency[point] if not visited[i])
        visited[i] = True
        f = faces[i]
        s, n = (int(v) for v in pinfo[f])
        segment = [tuple(p) for p in pv[s : s + n]]
        reverse = int(ends[f][1]) == point
        ring += segment[::-1] if reverse else segment
        point = int(ends[f][0]) if reverse else int(ends[f][1])
        ring.append(tuple(pc[point]))
    assert point == first
    return np.array([p for k, p in enumerate(ring) if k == 0 or p != ring[k - 1]])


def main():
    reference_dir, native_dir, report_path = sys.argv[1:4]
    reference = json.load(open(Path(reference_dir) / "manifest.json"))["entries"]
    native = {
        (e["fixture"], e["mesh"]): e
        for e in json.load(open(Path(native_dir) / "native_manifest.json"))["entries"]
    }
    report, failures = [], 0
    for entry in reference:
        fixture, mesh, stem = entry["fixture"], entry["mesh"], entry["stem"]
        ne = native[(fixture, mesh)]
        py_coords, py_off, py_cells = load(reference_dir, stem)
        rs_coords, rs_off, rs_cells = load(native_dir, stem)
        with h5py.File(entry["path"], "r") as f:
            g = f[f"Geometry/2D Flow Areas/{mesh}"]
            arrays = dict(
                cinfo=g["Cells Face and Orientation Info"][()],
                cvals=g["Cells Face and Orientation Values"][()],
                fpi=g["Faces FacePoint Indexes"][()],
                pinfo=g["Faces Perimeter Info"][()],
                pv=g["Faces Perimeter Values"][()],
                fpc=g["FacePoints Coordinate"][()][:, :2],
            )
        n_cells = len(arrays["cinfo"])
        face_counts = arrays["cinfo"][:, 1]
        result = {"fixture": fixture, "mesh": mesh, "cells": int(n_cells)}

        # (c) cell sets and omissions.
        py_set, rs_set = set(py_cells.tolist()), set(rs_cells.tolist())
        result["python_rings"], result["native_rings"] = len(py_set), len(rs_set)
        result["cell_sets_identical"] = bool(
            py_set == rs_set and len(py_set) == len(py_cells) and len(rs_set) == len(rs_cells)
        )
        omitted_py = sorted(set(range(n_cells)) - py_set)
        omitted_rs = sorted(set(range(n_cells)) - rs_set)
        result["omitted_identical"] = omitted_py == omitted_rs
        result["omitted"] = len(omitted_rs)
        py_diag = {
            d["cell_id"]: (d["reason_code"], d["face_count"])
            for d in entry["diagnostics"]
            if d["status"] != "ambiguous"
        }
        rs_diag = {}
        for d in ne["diagnostics"]:
            # Count omissions state "N faces"; geometry omissions carry free text, so the face
            # count comes from the HDF arrays (the Python side reports the same number).
            head = d["reason"].split()[0]
            faces = int(head) if d["reason_code"] != "no_polygon" else int(face_counts[d["cell_id"]])
            rs_diag[d["cell_id"]] = (d["reason_code"], faces)
        result["diagnostics_identical"] = py_diag == rs_diag
        result["diagnostics_cover_omitted"] = sorted(py_diag) == omitted_py == sorted(rs_diag)
        result["python_ambiguous_cells"] = sum(1 for d in entry["diagnostics"] if d["status"] == "ambiguous")
        # Omitted must be: < 3 faces, or an invalid reference ring (checked below).
        expected_omitted = {c for c in range(n_cells) if face_counts[c] < 3}
        extra = sorted(set(omitted_rs) - expected_omitted)
        result["omitted_ge3_faces"] = extra
        result["omitted_ge3_faces_ring_invalid"] = []
        for c in extra:
            r = reference_ring(arrays, c)
            ok = len(r) >= 4 and not shapely.is_valid(shapely.Polygon(r))
            result["omitted_ge3_faces_ring_invalid"].append(bool(ok))
        result["omitted_lt3_faces_all_omitted"] = expected_omitted <= set(omitted_rs)

        # (b) clockwise everywhere, and (d) WKB equality between the backends.
        py_rings, py_polys = polygons(py_coords, py_off)
        rs_rings, rs_polys = polygons(rs_coords, rs_off)
        result["python_ccw_rings"] = int(shapely.is_ccw(py_rings).sum())
        result["native_ccw_rings"] = int(shapely.is_ccw(rs_rings).sum())
        same_order = np.array_equal(py_cells, rs_cells)
        result["same_ring_order"] = bool(same_order)
        wkb_diff = None
        if same_order:
            wkb_diff = int(
                sum(a != b for a, b in zip(shapely.to_wkb(py_polys), shapely.to_wkb(rs_polys)))
            )
        result["wkb_differences"] = wkb_diff

        # (a) numpy reference against both backends.
        ref_mismatch_py = ref_mismatch_rs = 0
        ref_ccw = 0
        for row, cell in enumerate(rs_cells.tolist()):
            ref = reference_ring(arrays, cell)
            a = rs_coords[rs_off[row] : rs_off[row + 1]]
            if not np.array_equal(ref, a):
                ref_mismatch_rs += 1
            if cell in py_set:
                prow = int(np.searchsorted(py_cells, cell)) if same_order else int(np.where(py_cells == cell)[0][0])
                b = py_coords[py_off[prow] : py_off[prow + 1]]
                if not np.array_equal(ref, b):
                    ref_mismatch_py += 1
            if shapely.is_ccw(shapely.LinearRing(ref)):
                ref_ccw += 1
        result["numpy_reference_vs_native_mismatches"] = ref_mismatch_rs
        result["numpy_reference_vs_python_mismatches"] = ref_mismatch_py
        result["numpy_reference_ccw_rings"] = ref_ccw
        ok = (
            result["cell_sets_identical"]
            and result["omitted_identical"]
            and result["diagnostics_identical"]
            and result["diagnostics_cover_omitted"]
            and result["omitted_lt3_faces_all_omitted"]
            and all(result["omitted_ge3_faces_ring_invalid"])
            and result["python_ccw_rings"] == 0
            and result["native_ccw_rings"] == 0
            and ref_ccw == 0
            and wkb_diff == 0
            and ref_mismatch_py == 0
            and ref_mismatch_rs == 0
        )
        result["pass"] = bool(ok)
        failures += not ok
        report.append(result)
        print(json.dumps(result), flush=True)
    Path(report_path).write_text(json.dumps(report, indent=1))
    print("FAILURES", failures)
    sys.exit(1 if failures else 0)


main()
