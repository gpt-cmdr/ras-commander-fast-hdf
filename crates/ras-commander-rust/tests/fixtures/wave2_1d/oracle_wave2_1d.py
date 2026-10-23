"""Independent h5py oracle for the 1D (API 7) engine readers.

Usage: python -I oracle_wave2_1d.py <fixtures.json> <out-manifest.json>

fixtures.json is {"base": <dir>, "fixtures": [{"name", "path" (relative to base)}]}. The
manifest keeps `base` and relative paths; tests resolve them below RAS_WAVE2_1D_FIXTURE_ROOT
(default: base) and verify each SHA256, so the suite is relocatable to another host. Every source is opened read-only; its SHA256 is
recorded. For each family the oracle records shapes, stored dtypes, SHA256 of the exact
little-endian bytes of the values the engine must return, and deterministic samples. It does
not import the engine and implements the selection rules from the HDF datasets directly.
"""
import hashlib
import json
import sys

import h5py
import numpy as np

XS = "Geometry/Cross Sections"
UNSTEADY = "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series"
SUMMARY = "Results/Unsteady/Output/Output Blocks/Base Output/Summary Output"
STEADY = "Results/Steady/Output/Output Blocks/Base Output/Steady Profiles"
FIELDS = [
    ("station_elevation", "Station Elevation Info", "Station Elevation Values", False, True),
    ("mannings_n", "Manning's n Info", "Manning's n Values", False, True),
    ("ineffective_areas", "Ineffective Info", "Ineffective Blocks", True, True),
    ("deck_high", "Deck High Info", "Deck High Values", False, True),
    ("deck_low", "Deck Low Info", "Deck Low Values", False, True),
    ("property_table", "Property Tables/XSEC Info", "Property Tables/XSEC Value", False, True),
    ("property_cells", "Property Tables/Cell Info", "Property Tables/Cell Value", False, False),
]
LINES = [
    ("cross_section_cut_lines", XS, "Polyline"),
    ("river_centerlines", "Geometry/River Centerlines", "Polyline"),
    ("river_bank_lines", "Geometry/River Bank Lines", "Polyline"),
    ("river_edge_lines", "Geometry/River Edge Lines", "Polyline"),
    ("river_flow_paths", "Geometry/River Flow Paths", "Flow Path Lines"),
    ("structure_centerlines", "Geometry/Structures", "Centerline"),
    ("storage_area_polygons", "Geometry/Storage Areas", "Polygon"),
]
LEGACY = ("Node Names", "River Stations", "River Names", "Reach Names")


def sha(b):
    return hashlib.sha256(b).hexdigest()


def le(a):
    a = np.ascontiguousarray(a)
    if a.dtype.names is None and a.dtype.byteorder == ">":
        a = a.astype(a.dtype.newbyteorder("<"))
    return a.tobytes()


def text(v):
    if isinstance(v, bytes):
        return v.rstrip(b"\x00 ").decode("latin-1")
    return str(v)


def file_sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 22), b""):
            h.update(chunk)
    return h.hexdigest()


def raw_compound(ds):
    a = ds[()]
    assert a.dtype.itemsize == ds.id.get_type().get_size(), ds.name
    return a, a.tobytes()


def labels(ds, columns):
    found = [k for k in ds.attrs if k in ("Variables", "Variable_Unit", "Column", "Column=")]
    if not found:
        return None
    if len(found) > 1:
        return "corrupt"
    v = ds.attrs[found[0]]
    if found[0] in ("Variables", "Variable_Unit"):
        if v.shape != (columns, 2):
            return "corrupt"
        return [[text(a), text(b)] for a, b in v]
    v = np.atleast_1d(v)
    if v.shape != (columns,):
        return "corrupt"
    return [[text(a), None] for a in v]


def xs_geometry(f):
    if XS not in f or "Attributes" not in f[XS]:
        return {"status": "absent"}
    g = f[XS]
    if any(a in g for a in LEGACY):
        return {"status": "unsupported"}
    attrs, raw = raw_compound(g["Attributes"])
    n = attrs.shape[0]
    out = {"status": "supported", "sections": n, "attributes_sha": sha(raw), "fields": {}}
    out["identities"] = [
        [text(r["River"]), text(r["Reach"]), text(r["RS"])] for r in attrs
    ]
    for name, info_n, val_n, compound, per_section in FIELDS:
        if info_n not in g and val_n not in g:
            out["fields"][name] = {"status": "unavailable"}
            continue
        info = g[info_n][()]
        vals_ds = g[val_n]
        rec = {
            "status": "supported",
            "info_shape": list(info.shape),
            "info_dtype": info.dtype.str,
            "values_shape": list(vals_ds.shape),
            "info_sha": sha(le(info)),
        }
        ranges = [(int(r[0]), int(r[0]) + int(r[1])) for r in info]
        if compound:
            vals, _ = raw_compound(vals_ds)
            packed = b"".join(vals[s:e].tobytes() for s, e in ranges)
            rec["values_dtype"] = "compound"
            rec["labels"] = None
        else:
            vals = vals_ds[()]
            packed = b"".join(le(vals[s:e]) for s, e in ranges)
            rec["values_dtype"] = vals.dtype.str
            rec["labels"] = labels(vals_ds, vals.shape[1])
        rec["packed_sha"] = sha(packed)
        rec["packed_rows"] = int(sum(e - s for s, e in ranges))
        m = len(ranges)
        sample = [0, m // 2, m - 1, m // 2] if m else []
        rec["sample"] = sample
        if compound:
            rec["sample_sha"] = sha(b"".join(vals[ranges[i][0]:ranges[i][1]].tobytes() for i in sample))
        else:
            rec["sample_sha"] = sha(b"".join(le(vals[ranges[i][0]:ranges[i][1]]) for i in sample))
        rec["sample_ranges"] = [list(ranges[i]) for i in sample]
        # Leading records that are sections: all when counts agree; with extra XSEC Info rows,
        # rows 0..n-1 only if every first HTAB elevation equals that section's minimum stored
        # station elevation exactly (independent re-derivation from the raw datasets).
        if per_section and info.shape[0] == n:
            rec["section_records"] = n
        elif name == "property_table" and info.shape[0] > n:
            sei = g["Station Elevation Info"][()]
            sev = g["Station Elevation Values"][()]
            ok = all(
                int(sei[i][1]) > 0 and int(info[i][1]) > 0
                and np.float64(vals[int(info[i][0]), 0]).tobytes()
                == np.float64(sev[int(sei[i][0]) : int(sei[i][0]) + int(sei[i][1]), 1].min()).tobytes()
                for i in range(n)
            )
            rec["section_records"] = n if ok else 0
            rec["trailing_rows"] = int(info.shape[0] - n)
        else:
            rec["section_records"] = 0
        out["fields"][name] = rec
    consumed = {"Attributes", "Polyline Info", "Polyline Parts", "Polyline Points"}
    for _, i, v, _, _ in FIELDS:
        consumed.update([i, v])
    unq = []

    def visit(name, obj):
        if isinstance(obj, h5py.Dataset) and name not in consumed:
            unq.append(f"{XS}/{name}")

    g.visititems(visit)
    out["unqualified"] = sorted(unq)
    return out


def lines(f):
    out = {}
    for fam, group, prefix in LINES:
        if group not in f or f"{prefix} Info" not in f[group]:
            out[fam] = {"status": "unavailable"}
            continue
        g = f[group]
        info = g[f"{prefix} Info"][()]
        parts = g[f"{prefix} Parts"][()]
        pts = g[f"{prefix} Points"][()]
        rec = {
            "status": "supported",
            "features": int(info.shape[0]),
            "info_sha": sha(le(info)),
            "parts_sha": sha(le(parts)),
            "points_sha": sha(le(pts)),
            "points_dtype": pts.dtype.str,
            "has_attributes": "Attributes" in g,
        }
        if "Attributes" in g:
            rec["attributes_sha"] = sha(raw_compound(g["Attributes"])[1])
        ranges = []
        for i in info:
            ps, pc, qs, qc = (int(x) for x in i)
            ranges.append([ps, ps + pc, [[ps + int(parts[q][0]), ps + int(parts[q][0]) + int(parts[q][1])] for q in range(qs, qs + qc)]])
        rec["ranges_sha"] = sha(json.dumps(ranges).encode())
        out[fam] = rec
    return out


def reaches(f):
    p = "Geometry/River Centerlines/Attributes"
    if p not in f:
        return None
    a = f[p][()]
    cols = a.dtype.names

    def opt(r, c):
        return text(r[c]) if c in cols else None

    return [
        [text(r["River Name"]), text(r["Reach Name"]), opt(r, "US Type"), opt(r, "US Name"), opt(r, "DS Type"), opt(r, "DS Name")]
        for r in a
    ]


def points(f, group):
    if group not in f or "Attributes" not in f[group]:
        return None
    a, raw = raw_compound(f[group]["Attributes"])
    p = f[group]["Points"][()]
    names = [text(r["Name"]) for r in a] if "Name" in a.dtype.names else None
    return {"rows": int(p.shape[0]), "attributes_sha": sha(raw), "points_sha": sha(le(p)), "names": names}


def surfaces(f):
    g = f.get("Geometry/Cross Section Interpolation Surfaces")
    if g is None or "TIN Info" not in g:
        return None
    info = g["TIN Info"][()]
    rec = {
        "surfaces": int(info.shape[0]),
        "info_sha": sha(le(info)),
        "points_sha": sha(le(g["TIN Points"][()])),
        "triangles_sha": sha(le(g["TIN Triangles"][()])),
        "per_surface": {n: sha(le(g[n][()])) for n in ["XSIDs", "Areas", "Orthogonal Vectors", "Bounding Lines"] if n in g},
    }
    return rec


def main():
    spec = json.load(open(sys.argv[1]))
    base = spec["base"]
    cases = []
    for fx in spec["fixtures"]:
        path = base.rstrip("/") + "/" + fx["path"]
        case = {"name": fx["name"], "path": fx["path"], "sha256": file_sha(path)}
        with h5py.File(path, "r") as f:
            case["xs_geometry"] = xs_geometry(f)
            case["lines"] = lines(f)
            case["reaches"] = reaches(f)
            case["junctions"] = points(f, "Geometry/Junctions")
            case["surfaces"] = surfaces(f)
            for extra in EXTRA:
                extra(f, case)
        cases.append(case)
        print(fx["name"], "ok", flush=True)
    json.dump({"generator": "oracle_wave2_1d.py", "base": base, "h5py": h5py.__version__, "numpy": np.__version__, "cases": cases}, open(sys.argv[2], "w"), indent=1)


def xs_identity_ok(f, mode_root, geo):
    """Results-side River/Reach/Station equal to geometry (trimmed), compound or packed."""
    if mode_root == "unsteady":
        base = UNSTEADY + "/Cross Sections"
        attrs_p, only_p = base + "/Cross Section Attributes", base + "/Cross Section Only"
    else:
        attrs_p = "Results/Steady/Output/Geometry Info/Cross Section Attributes"
        only_p = "Results/Steady/Output/Geometry Info/Cross Section Only"
    if attrs_p in f:
        a = f[attrs_p][()]
        got = [[text(r["River"]).strip(), text(r["Reach"]).strip(), text(r["Station"]).strip()] for r in a]
    elif only_p in f:
        got = []
        for s in f[only_p][()]:
            s = s.decode("latin-1")
            got.append([s[:16].strip(), s[17:33].strip(), s[34:].strip()])
    else:
        return False
    return got == [[a.strip(), b.strip(), c.strip()] for a, b, c in geo]


def ms_axis(f):
    p = UNSTEADY + "/Time Date Stamp (ms)"
    if p not in f:
        return None
    import datetime as dt
    out = []
    for s in f[p][()]:
        s = s.decode()
        day, hms = s.split(" ", 1)
        h, m, sec, ms = (int(x) for x in hms.split(":"))
        d = dt.datetime.strptime(day.title(), "%d%b%Y")
        d = d + dt.timedelta(hours=h, minutes=m, seconds=sec, milliseconds=ms)
        out.append(int((d - dt.datetime(1970, 1, 1)).total_seconds() * 1000))
    return out


def xs_results_case(f, case):
    geo = case["xs_geometry"].get("identities")
    res = {}
    for mode, base in (("steady", STEADY), ("unsteady", UNSTEADY)):
        g = base + "/Cross Sections"
        if g not in f:
            res[mode] = {"status": "unavailable"}
            continue
        ok = geo is not None and xs_identity_ok(f, mode, geo)
        n = len(geo) if geo else 0
        if mode == "steady":
            rows = f[STEADY + "/Profile Names"].shape[0]
        else:
            rows = f[UNSTEADY + "/Time"].shape[0]
        rec = {"status": "supported" if ok else "identity_failed", "rows": rows, "variables": {}}
        names = []

        def visit(name, obj):
            if isinstance(obj, h5py.Dataset) and name not in ("Cross Section Attributes", "Cross Section Only"):
                names.append(name)

        f[g].visititems(visit)
        for name in sorted(names):
            ds = f[g][name]
            a = ds[()]
            v = {"shape": list(a.shape), "dtype": a.dtype.str, "sha": sha(le(a)), "attrs": sorted(ds.attrs.keys())}
            if a.ndim == 2 and a.shape[1] == n and n:
                v["domain"] = "cross_section"
            elif a.ndim == 3 and a.shape[2] == n and n:
                v["domain"] = "cross_section_components"
            else:
                v["domain"] = "other"
            v["readable"] = ok and v["domain"] != "other" and a.shape[0] == rows and rows > 0
            if v["domain"] == "cross_section_components":
                dim = [text(x).strip() for x in np.atleast_1d(ds.attrs.get("Dimension", []))]
                ok3 = mode == "steady" and dim == ["Profiles", "Variables", "Cross Sections"]
                v["readable"] = v["readable"] and ok3
            if v["readable"]:
                step = max(1, rows // 7)
                cols = [n - 1, 0, n // 2, n // 2]
                sub = a[0:rows:step]
                v["sample_rows"] = [0, rows, step]
                v["sample_cols"] = cols
                v["sample_sha"] = sha(le(sub[..., cols]))
                k = min(n, 5)
                v["contig_sha"] = sha(le(a[rows // 2 : rows, ..., 0:k]))
                v["contig"] = [rows // 2, rows, k]
            rec["variables"][name] = v
        if mode == "unsteady":
            ms = ms_axis(f)
            if ms and len(ms) > 2:
                rec["window"] = [ms[1], ms[len(ms) // 2], 1, len(ms) // 2 + 1]
        res[mode] = rec
    case["xs_results"] = res


def summary_rows(attrs, rows):
    names = [k for k in attrs if k in ("Variable_Unit", "Variables")]
    if names:
        v = attrs[names[0]]
        if v.shape == (rows, 2) and rows > 1:
            return [[text(a), text(b)] for a, b in v]
    out = [None] * rows
    units = text(attrs["Units"]) if "Units" in attrs else None
    anyl = False
    for k in attrs:
        if k.startswith("Row ") and ":" in k:
            i = int(k[4:].split(":")[0].strip())
            if i < rows:
                out[i] = [k.split(":", 1)[1].strip(), text(attrs[k])]
                anyl = True
        elif k.startswith("Row") and k[3:].isdigit():
            i = int(k[3:])
            if i < rows:
                out[i] = [text(attrs[k]), units]
                anyl = True
    if not anyl or any(x is None for x in out):
        return None
    return out


def summary_case(f, case):
    if SUMMARY not in f:
        case["summary"] = None
        return
    geo = case["xs_geometry"].get("identities")
    xs_ok = geo is not None and xs_identity_ok(f, "unsteady", geo)
    sa = f["Geometry/Storage Areas/Attributes"].shape[0] if "Geometry/Storage Areas/Attributes" in f else None
    out = []
    for fam in sorted(f[SUMMARY].keys()):
        if fam == "2D Flow Areas":
            continue
        count = len(geo) if (fam == "Cross Sections" and xs_ok) else (sa if fam == "Storage Areas" else None)
        names = []
        f[SUMMARY][fam].visititems(lambda n, o: names.append(n) if isinstance(o, h5py.Dataset) else None)
        for name in sorted(names):
            ds = f[SUMMARY][fam][name]
            a = ds[()]
            axis = None
            declared = None
            for k in ("Variable_Unit", "Variables"):
                if k in ds.attrs:
                    declared = ds.attrs[k].shape[0]
            if count is not None:
                if a.ndim == 1 and a.shape[0] == count:
                    axis = 0
                elif a.ndim == 2 and a.shape[0] == count and a.shape[1] == count:
                    axis = 1 if (declared == a.shape[0] and declared > 1) else None
                elif a.ndim == 2 and a.shape[1] == count:
                    axis = 1
                elif a.ndim == 2 and a.shape[0] == count:
                    axis = 0
            rl = summary_rows(ds.attrs, a.shape[0]) if axis == 1 else None
            rec = {
                "family": fam, "name": name, "shape": list(a.shape), "dtype": a.dtype.str,
                "sha": sha(le(a)), "axis": axis, "row_labels": rl,
                "time_rows": [i for i, l in enumerate(rl or []) if l[0].startswith("Time")],
            }
            if axis is not None:
                n = a.shape[axis]
                sel = [n - 1, 0, n // 2]
                rec["sample"] = sel
                rec["sample_sha"] = sha(le(a[sel] if axis == 0 else a[:, sel]))
            out.append(rec)
    case["summary"] = out


EXTRA = [xs_results_case, summary_case]


STRUCT_KINDS = [
    ("inline_structure", "Inline Structures", "Inline", ("unsteady", "steady")),
    ("bridge", "Bridges", "Bridge", ("unsteady", "steady")),
    ("culvert", "Culverts", "Culvert", ("unsteady", "steady")),
    ("pump_station", "Pumping Stations", None, ("unsteady", "steady")),
    ("lateral_structure", "Lateral Structures", "Lateral", ("steady",)),
    ("storage_area", "Storage Areas", None, ("steady",)),
]


def group_dump(f, root):
    if root not in f or not isinstance(f[root], h5py.Group):
        return None
    tables, arrays, other = {}, {}, []

    def visit(name, obj):
        if not isinstance(obj, h5py.Dataset):
            return
        dt = obj.dtype
        if dt.names is not None:
            tables[name] = sha(raw_compound(obj)[1])
        elif dt.kind in "iuf":
            arrays[name] = sha(le(obj[()]))
        else:
            other.append(name)

    f[root].visititems(visit)
    return {"tables": tables, "arrays": arrays, "other": sorted(other)}


def structure_geometry_case(f):
    p = "Geometry/Structures/Attributes"
    if p not in f:
        return None
    a, raw = raw_compound(f[p])
    names = a.dtype.names
    recs = []
    ti = f["Geometry/Structures/Table Info"][()] if "Geometry/Structures/Table Info" in f else None
    for i, r in enumerate(a):
        rec = [text(r["Type"]) if "Type" in names else None, text(r["River"]), text(r["Reach"]), text(r["RS"])]
        ranges = []
        if ti is not None:
            for c in ti.dtype.names:
                if c.endswith(" (Index)"):
                    base = c[: -len(" (Index)")]
                    s, n = int(ti[i][c]), int(ti[i][base + " (Count)"])
                    ranges.append([base, s, s + n])
        recs.append([rec, ranges])
    return {"rows": int(a.shape[0]), "attributes_sha": sha(raw), "records": recs}


def series_labels(ds, columns):
    found = [k for k in ds.attrs if k in ("Variable_Unit", "Connections")]
    if not found:
        return None
    if len(found) > 1:
        return "corrupt"
    v = ds.attrs[found[0]]
    if found[0] == "Variable_Unit":
        return [[text(x), text(y)] for x, y in v] if v.shape == (columns, 2) else "corrupt"
    v = np.atleast_1d(v)
    return [[text(x), None] for x in v] if v.shape == (columns,) else "corrupt"


def structure_results_case(f, case):
    out = {}
    geo = f["Geometry/Structures/Attributes"][()] if "Geometry/Structures/Attributes" in f else None
    for kind, gname, ty, modes in STRUCT_KINDS:
        for mode in modes:
            base = UNSTEADY if mode == "unsteady" else STEADY
            fam = f"{base}/{gname}"
            key = f"{kind}/{mode}"
            if fam not in f:
                out[key] = {"status": "unavailable"}
                continue
            if (mode == "steady") != (kind in ("lateral_structure", "storage_area")):
                # No real fixture qualifies this (kind, mode) layout.
                out[key] = {"status": "unqualified"}
                continue
            rows = f[UNSTEADY + "/Time"].shape[0] if mode == "unsteady" else f[STEADY + "/Profile Names"].shape[0]
            groups = sorted(k for k in f[fam] if isinstance(f[fam][k], h5py.Group))
            if ty is not None:
                keys = [] if geo is None else [
                    (i, f"{text(r['River']).strip()} {text(r['Reach']).strip()} {text(r['RS']).strip()}")
                    for i, r in enumerate(geo) if text(r["Type"]).strip() == ty
                ]
                table = "Geometry/Structures/Attributes"
            else:
                table = "Geometry/Pump Stations/Attributes" if kind == "pump_station" else "Geometry/Storage Areas/Attributes"
                keys = [(i, text(n).strip()) for i, n in enumerate(f[table]["Name"])] if table in f else []
            ids = []
            ok = True
            for g in groups:
                m = [i for i, k in keys if k == g.strip()]
                if len(m) != 1:
                    ok = False
                ids.append(m[0] if len(m) == 1 else None)
            rec = {"status": "supported" if ok else "identity", "rows": rows, "groups": groups, "identity_rows": ids, "table": table, "structures": {}}
            for g in groups:
                series, statics = [], []

                def walk(grp, rel, depth):
                    for n in sorted(grp.keys()):
                        o = grp[n]
                        r = f"{rel}/{n}" if rel else n
                        if isinstance(o, h5py.Group):
                            if depth == 0 and n == "Geometric Info":
                                continue
                            walk(o, r, depth + 1)
                        elif o.ndim == 1:
                            statics.append(r)
                        else:
                            a = o[()]
                            step = max(1, rows // 5)
                            series.append({
                                "relative": r, "shape": list(a.shape), "dtype": a.dtype.str,
                                "sha": sha(le(a)), "labels": series_labels(o, a.shape[1]),
                                "sample": [0, rows, step], "sample_sha": sha(le(a[0:rows:step])),
                            })

                walk(f[fam][g], "", 0)
                rec["structures"][g] = {"series": sorted(series, key=lambda s: s["relative"]), "statics": statics}
            rec["family_series"] = {
                n: sha(le(f[fam][n][()])) for n in f[fam] if isinstance(f[fam][n], h5py.Dataset)
            }
            out[key] = rec
    case["structure_results"] = out
    case["structure_geometry"] = structure_geometry_case(f)
    case["geometry_groups"] = {
        "structures": group_dump(f, "Geometry/Structures"),
        "pump_stations": group_dump(f, "Geometry/Pump Stations"),
        "storage_areas": group_dump(f, "Geometry/Storage Areas"),
    }
    case["pump_stations"] = points(f, "Geometry/Pump Stations")


EXTRA.append(structure_results_case)


if __name__ == "__main__":
    main()
