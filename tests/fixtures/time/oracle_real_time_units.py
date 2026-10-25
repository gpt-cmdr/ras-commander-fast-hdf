"""h5py oracle for real-fixture time axes and units. Run on the machine holding the immutable
fixtures:  python -I oracle_real_time_units.py FIXTURE_DIR OUT.json [name-substring ...]
Defaults select the BaldEagleDamBrk p07 / p18 result HDF files, the public FEMA eBFE
AustinOyster.p03 (HEC-RAS 5.0.7 2D) plan and the eBFE-derived Stream248.p02 (HEC-RAS 7.0.1
1D unsteady) plan. Never modifies the inputs. The Rust test
(tests/time_units_conformance.rs, ignored) reads OUT.json via RAS_TIME_ORACLE."""
import calendar, datetime, hashlib, json, sys
from pathlib import Path
import h5py, numpy as np

BASE = 'Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series'
STEADY = 'Results/Steady/Output/Output Blocks/Base Output/Steady Profiles'
MON = ['JAN','FEB','MAR','APR','MAY','JUN','JUL','AUG','SEP','OCT','NOV','DEC']

def oracle_ms(text):
    d, t = text.split()
    parts = [int(x) for x in t.split(':')]
    h, m, s = (parts + [0, 0])[:3]
    msec = parts[3] if len(parts) > 3 else 0
    extra = 0
    if h == 24:
        h, extra = 0, 86400
    dt = datetime.datetime(int(d[5:]), MON.index(d[2:5].upper()) + 1, int(d[:2]), h, m, s)
    return (calendar.timegm(dt.timetuple()) + extra) * 1000 + msec

def text_of(v):
    if isinstance(v, np.ndarray):
        if v.size != 1:
            return None
        v = v.reshape(-1)[0]
    if isinstance(v, (bytes, np.bytes_)):
        try:
            return bytes(v).rstrip(b'\0 ').decode('utf-8')
        except UnicodeDecodeError:
            return None
    if isinstance(v, str):
        return v
    return None

def attr_record(obj, name):
    if name not in obj.attrs:
        return {'present': False}
    v = obj.attrs[name]
    raw = v.tobytes() if hasattr(v, 'tobytes') else None
    return {'present': True, 'text': text_of(v), 'raw_hex': raw.hex() if raw else None}

def axis(f):
    g = f.get(BASE)
    out = {'present': g is not None}
    if g is None:
        return out
    if 'Time' in g:
        a = g['Time'][()]
        out['days'] = [float(x) for x in a.astype(np.float64)]  # display only
        # exact IEEE-754 bits of the value widened to f64 (f32 widening is exact)
        out['days_bits'] = [int(x) for x in a.astype(np.float64).view(np.uint64)]
        out['days_dtype'] = str(g['Time'].dtype)
    if 'Time Date Stamp' in g:
        raw = g['Time Date Stamp'][()]
        out['strings'] = [bytes(x).rstrip(b'\0 ').decode('ascii') for x in raw]
        out['string_ms'] = [oracle_ms(s) for s in out['strings']]
    if 'Time Date Stamp (ms)' in g:
        ds = g['Time Date Stamp (ms)']
        a = ds[()]
        out['ms_dtype'] = str(ds.dtype)
        if ds.dtype.kind in ('S', 'O', 'U'):
            # real HEC form: strings such as 01JAN1999 12:00:00:000
            out['ms_strings'] = [(x.decode('ascii') if isinstance(x, bytes) else str(x)).rstrip('\x00 ') for x in a]
            ms = [oracle_ms(x) for x in out['ms_strings']]
            out['ms_string_ms'] = ms
        else:
            ms = [int(x) for x in a]
            out['ms'] = ms
        # integer ms has no proven epoch and is not compared; string ms is compared with the
        # whole-second Time Date Stamp axis at second resolution (floor).
        if 'string_ms' in out and 'ms_string_ms' in out:
            out['ms_matches_strings'] = [a // 1000 for a in ms] == [b // 1000 for b in out['string_ms']]
    best = out.get('ms_string_ms') or out.get('string_ms')
    if best and 'days' in out:
        d = out['days']
        f32 = out.get('days_dtype', '').endswith('f4')
        tol = 1.5 + max(abs(x) for x in d) * (4 * 1.1920929e-07 if f32 else 1e-12) * 86400000
        dev = max(abs((d[i] - d[0]) * 86400000 - (best[i] - best[0])) for i in range(len(d)))
        out['days_max_dev_ms'] = dev
        out['days_consistent'] = dev <= tol
    rows = {}
    for p in [BASE + '/Cross Sections/Water Surface', BASE + '/Cross Sections/Flow']:
        if p in f:
            rows[p] = int(f[p].shape[0])
    if BASE + '/2D Flow Areas' in f:
        for a in f[BASE + '/2D Flow Areas']:
            p = '%s/2D Flow Areas/%s/Water Surface' % (BASE, a)
            if p in f:
                rows[p] = int(f[p].shape[0])
    out['result_rows'] = rows
    return out

LABELS = ['WSEL', 'Velocity', 'Lateral Flow', 'Cumulative Volumetric Flow', 'Depth',
          'Face Velocity', 'Flow', 'Water Surface']
MESH_VARS = ['Water Surface', 'Depth', 'Velocity', 'Face Velocity']
XS_VARS = ['Water Surface', 'Flow', 'Flow Lateral', 'Flow Volume Cumulative',
           'Velocity Channel', 'Velocity Total']

def texts(v):
    """Decoded text list for a string-valued attribute, else None."""
    def one(x):
        if isinstance(x, str):
            return x
        try:
            return bytes(x).rstrip(bytes([0, 32])).decode('utf-8')
        except UnicodeDecodeError:
            return None
    if isinstance(v, (bytes, np.bytes_, str)):
        t = one(v)
        return None if t is None else [t]
    if isinstance(v, np.ndarray) and v.dtype.kind in ('S', 'O', 'U'):
        out = [one(x) for x in v.reshape(-1)]
        return None if any(x is None for x in out) else out
    return None

def all_attrs(obj):
    rec = {}
    for name in obj.attrs:
        v = obj.attrs[name]
        arr = np.asarray(v)
        rec[name] = {'shape': list(arr.shape), 'dtype': str(arr.dtype), 'texts': texts(v)}
    return rec

def expected_units(attrs):
    """Independent priority: Units, Variable Units, Variable pair, one known label."""
    def scalar(name):
        r = attrs[name]
        return r['texts'][0] if r['texts'] is not None and r['shape'] == [] else None
    qn = scalar('Variable Name') if 'Variable Name' in attrs else None
    for name, src in [('Units', 'UnitsAttribute'), ('Variable Units', 'VariableUnits')]:
        if name in attrs:
            t = scalar(name)
            return {'source': src, 'status': 'declared' if t is not None else 'unsupported',
                    'text': t, 'quantity_name': qn}
    if 'Variable' in attrs:
        r = attrs['Variable']
        ok = r['texts'] is not None and r['shape'] == [2]
        return {'source': 'VariablePair', 'status': 'declared' if ok else 'unsupported',
                'text': r['texts'][1] if ok else None,
                'quantity_name': qn if qn is not None else (r['texts'][0] if ok else None)}
    found = [l for l in LABELS if l in attrs]
    if len(found) == 1:
        t = scalar(found[0])
        return {'source': 'QuantityLabel', 'attribute': found[0],
                'status': 'declared' if t is not None else 'unsupported', 'text': t,
                'quantity_name': qn}
    if found:
        return {'source': 'None', 'status': 'unsupported', 'text': None, 'quantity_name': qn}
    return {'source': 'None', 'status': 'absent', 'text': None, 'quantity_name': qn}

def units(f):
    vars_ = []
    a2 = BASE + '/2D Flow Areas'
    other = []
    if a2 in f:
        for area in f[a2]:
            for v in f[a2][area]:
                p = '%s/%s/%s' % (a2, area, v)
                if v not in MESH_VARS:
                    other.append(p)
                    continue
                attrs = all_attrs(f[p])
                vars_.append({'dataset': p, 'family': 'Mesh2D', 'area': area, 'variable': v,
                              'attrs': attrs, 'expected': expected_units(attrs)})
    for fam, base in [('CrossSectionUnsteady', BASE), ('CrossSectionSteady', STEADY)]:
        for v in XS_VARS:
            p = '%s/Cross Sections/%s' % (base, v)
            if p in f:
                attrs = all_attrs(f[p])
                vars_.append({'dataset': p, 'family': fam, 'area': None, 'variable': v,
                              'attrs': attrs, 'expected': expected_units(attrs)})
    system = [{'source': s, 'value': attr_record(f[s], 'Units System')}
              for s in ['/', '/Geometry'] if s in f]
    return {'variables': vars_, 'unit_system': system, 'unlisted_2d_datasets': other}

REQUIRED = ['.p07.', '.p18.', 'AustinOyster.p03', 'Stream248.p02']

def main():
    root = Path(sys.argv[1])
    wanted = sys.argv[3:] or REQUIRED
    cases = []
    for p in sorted(root.rglob('*.hdf')):
        if not any(w in p.name for w in wanted):
            continue
        digest = hashlib.sha256()
        with open(p, 'rb') as fh:
            for block in iter(lambda: fh.read(1 << 22), b''):
                digest.update(block)
        with h5py.File(p, 'r') as f:
            if BASE not in f:
                continue
            cases.append({'path': str(p.relative_to(root)).replace(chr(92), '/'),
                          'sha256': digest.hexdigest(),
                          'axis': axis(f), 'units': units(f),
                          'h5py': h5py.__version__})
    for token in wanted:
        if not any(token in c['path'] for c in cases):
            sys.exit('oracle: no file matched required token ' + token)
    for c in cases:
        if 'Stream248.p02' in c['path']:
            xs = [v for v in c['units']['variables'] if v['family'] == 'CrossSectionUnsteady']
            assert xs, 'Stream248.p02: expected 1D unsteady XS result datasets'
            for v in xs:
                if v['variable'] in ('Water Surface', 'Flow'):
                    assert v['expected']['status'] == 'declared', 'Stream248.p02: ' + v['dataset']
            assert any('Cross Sections' in k for k in c['axis']['result_rows']), 'no XS row count'
    Path(sys.argv[2]).write_text(json.dumps(cases, indent=1) + '\n')
    print(len(cases), 'files')

main()
