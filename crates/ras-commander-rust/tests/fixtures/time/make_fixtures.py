"""Generate synthetic time/units fixtures; run as: python -I make_fixtures.py OUTDIR
Needs h5py/numpy only (not needed at Rust test runtime). Writes *.h5 and manifest.json
containing SHA256 and the independent expectations (computed here with datetime/calendar,
never by the Rust parser)."""
import calendar, datetime, hashlib, json, sys
from pathlib import Path
import h5py, numpy as np

BASE = 'Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series'
MON = ['JAN','FEB','MAR','APR','MAY','JUN','JUL','AUG','SEP','OCT','NOV','DEC']

def oracle_ms(text):
    """Independent HEC stamp -> civil ms since 1970 (24:00 = next day 00:00)."""
    d, t = text.split()
    day, mon, year = int(d[:2]), MON.index(d[2:5].upper()) + 1, int(d[5:])
    f = [int(x) for x in t.split(':')]
    h, m, s = f[:3]
    msec = f[3] if len(f) > 3 else 0
    extra = 0
    if h == 24:
        h, extra = 0, 86400
    dt = datetime.datetime(year, mon, day, h, m, s)  # raises on impossible dates
    return (calendar.timegm(dt.timetuple()) + extra) * 1000 + msec

def fixed(strings, width=None):
    width = width or max(len(s) for s in strings)
    return np.array([s.encode() for s in strings], dtype='S%d' % width)

def make(path, strings=None, ms=None, days=None, days_dtype='<f8', rows_result=None,
         ms_dtype='<i8', units=None):
    with h5py.File(path, 'w', libver='earliest') as f:
        g = f.create_group(BASE)
        if days is not None:
            g.create_dataset('Time', data=np.array(days, dtype=days_dtype))
        if strings is not None:
            g.create_dataset('Time Date Stamp', data=fixed(strings))
        if ms is not None:
            if isinstance(ms[0], str):  # real HEC form: fixed strings DDMONYYYY HH:MM:SS:mmm
                g.create_dataset('Time Date Stamp (ms)', data=fixed(ms))
            else:
                g.create_dataset('Time Date Stamp (ms)', data=np.array(ms, dtype=ms_dtype))
        if rows_result is not None:
            g.create_dataset('Cross Sections/Water Surface',
                             data=np.zeros((rows_result, 2), dtype='<f4'))
        if units is not None:
            units(f)

def text_attr(obj, name, raw, utf8=False):
    """Fixed-length string attribute from exact bytes (ASCII or UTF-8 cset)."""
    if utf8:
        dt = h5py.string_dtype('utf-8', len(raw))
    else:
        dt = h5py.string_dtype('ascii', len(raw))
    obj.attrs.create(name, np.array(raw, dtype=dt), dtype=dt)

def units_full(f):
    text_attr(f, 'Units System', b'US Customary')
    f.create_group('Geometry').attrs['dummy'] = np.int32(0)
    text_attr(f['Geometry'], 'Units System', b'SI Metric')
    area = f.create_group(BASE + '/2D Flow Areas/Perimeter 1')
    for name, u in [('Water Surface', b'ft'), ('Depth', b'ft'), ('Velocity', None),
                    ('Face Velocity', 'm³/s'.encode('utf-8'))]:
        d = area.create_dataset(name, data=np.zeros((4, 3), dtype='<f4'))
        if u is not None:
            text_attr(d, 'Units', u, utf8=name == 'Face Velocity')
    xs = f.create_group(BASE + '/Cross Sections')
    d = xs.create_dataset('Water Surface', data=np.zeros((4, 2), dtype='<f4'))
    text_attr(d, 'Units', b'ft')
    d = xs.create_dataset('Flow', data=np.zeros((4, 2), dtype='<f8'))
    text_attr(d, 'Units', 'ft³/s'.encode('utf-8'), utf8=True)

def pair(obj, name, items):
    obj.attrs.create(name, np.array([i.encode() for i in items], dtype='S32'))

def minmax(d):
    d.attrs.create('Maximum Value of Data Set', np.float32(9.5))
    d.attrs.create('Minimum Value of Data Set', np.float32(1.25))

def units_real_layouts(f):
    """Layouts observed in real HEC-RAS 6.x files: no attribute is named Units."""
    xs = f.create_group(BASE + '/Cross Sections')
    d = xs.create_dataset('Water Surface', data=np.zeros((3, 2), dtype='<f4'))
    pair(d, 'Variable', ['Stage', 'Feet'])
    text_attr(d, 'Variable Name', b'Stage')
    text_attr(d, 'Variable Units', b'Feet')
    minmax(d)
    d = xs.create_dataset('Flow', data=np.zeros((3, 2), dtype='<f4'))
    pair(d, 'Variable', ['Flow', 'cfs'])
    text_attr(d, 'Variable Name', b'Flow')
    text_attr(d, 'Variable Units', b'cfs')
    for name, attr, val in [('Flow Lateral', 'Lateral Flow', b'cfs'),
                            ('Flow Volume Cumulative', 'Cumulative Volumetric Flow', b'Feet^3'),
                            ('Velocity Channel', 'Velocity', b'ft/s'),
                            ('Velocity Total', 'Velocity', b'ft/s')]:
        d = xs.create_dataset(name, data=np.zeros((3, 2), dtype='<f4'))
        text_attr(d, attr, val)
    xs.create_dataset('Cross Section Only', data=np.zeros((2, 2), dtype='<f4'))
    area = f.create_group(BASE + '/2D Flow Areas/D601')
    d = area.create_dataset('Water Surface', data=np.zeros((3, 4), dtype='<f4'))
    text_attr(d, 'WSEL', b'Feet')
    minmax(d)
    d = area.create_dataset('Depth', data=np.zeros((3, 4), dtype='<f4'))
    text_attr(d, 'Depth', b'Feet')
    d = area.create_dataset('Face Velocity', data=np.zeros((3, 5), dtype='<f4'))
    text_attr(d, 'Face Velocity', b'ft/s')
    area.create_dataset('Velocity', data=np.zeros((3, 4), dtype='<f4'))

def units_priority_edges(f):
    xs = f.create_group(BASE + '/Cross Sections')
    d = xs.create_dataset('Water Surface', data=np.zeros((2, 2), dtype='<f4'))
    text_attr(d, 'Units', b'm')
    text_attr(d, 'Variable Units', b'Feet')
    d = xs.create_dataset('Flow', data=np.zeros((2, 2), dtype='<f4'))
    text_attr(d, 'Variable Units', b'cfs')
    pair(d, 'Variable', ['Flow', 'other'])
    text_attr(d, 'Flow', b'label')
    d = xs.create_dataset('Flow Lateral', data=np.zeros((2, 2), dtype='<f4'))
    pair(d, 'Variable', ['Lateral', 'cfs'])
    d = xs.create_dataset('Velocity Channel', data=np.zeros((2, 2), dtype='<f4'))
    pair(d, 'Variable', ['a', 'b', 'c'])
    d = xs.create_dataset('Velocity Total', data=np.zeros((2, 2), dtype='<f4'))
    text_attr(d, 'Velocity', b'ft/s')
    text_attr(d, 'Lateral Flow', b'cfs')
    d = xs.create_dataset('Flow Volume Cumulative', data=np.zeros((2, 2), dtype='<f4'))
    d.attrs.create('Cumulative Volumetric Flow', np.int32(5))

def units_latin1(f):
    xs = f.create_group(BASE + '/Cross Sections')
    d = xs.create_dataset('Water Surface', data=np.zeros((2, 2), dtype='<f4'))
    # ASCII-declared storage holding a non-ASCII byte: must stay Unsupported with raw bytes
    text_attr(d, 'Units', b'\xb0F', utf8=False)
    d = xs.create_dataset('Flow', data=np.zeros((2, 2), dtype='<f4'))
    d.attrs.create('Units', np.int32(7))
    text_attr(f, 'Units System', b'Imperial')

def units_missing(f):
    xs = f.create_group(BASE + '/Cross Sections')
    xs.create_dataset('Water Surface', data=np.zeros((2, 2), dtype='<f4'))

ROLL = ['31DEC2019 22:00:00', '31DEC2019 23:00:00', '31DEC2019 24:00:00', '01JAN2020 01:00:00']
LEAP = ['28FEB2020 23:00:00', '29FEB2020 00:00:00', '29FEB2020 24:00:00', '01MAR2020 12:30:00']
MONTHS = ['15%s2021 06:00:00' % m for m in MON]

def elapsed(strings):
    m = [oracle_ms(x) for x in strings]
    return [(v - m[0]) / 86400000 for v in m]

def ms_strings(strings):
    return [x + ':000' for x in strings]

def ms_of(strings):
    return [oracle_ms(s) for s in strings]

# name -> (builder kwargs, expectation). expect = ok | {error: Unsupported|Corrupt|NotFound}
def cases():
    c = {}
    c['rollover_2400_with_ms'] = (dict(strings=ROLL, ms=ms_of(ROLL), days=[0, 1/24, 2/24, 3/24],
                                       days_dtype='<f4', rows_result=4),
                                  dict(ok=True, strings=ROLL, ms=ms_of(ROLL), ms_source='DerivedFromStrings',
                                       hour_2400_rows=[2], raw_ms=ms_of(ROLL), integer_ms_unsupported=True))
    c['rollover_2400_strings_only'] = (dict(strings=ROLL),
                                       dict(ok=True, strings=ROLL, ms=ms_of(ROLL),
                                            ms_source='DerivedFromStrings', hour_2400_rows=[2]))
    c['leap_day'] = (dict(strings=LEAP, ms=ms_of(LEAP)),
                     dict(ok=True, strings=LEAP, ms=ms_of(LEAP), ms_source='DerivedFromStrings',
                          hour_2400_rows=[2], raw_ms=ms_of(LEAP), integer_ms_unsupported=True))
    c['month_names'] = (dict(strings=MONTHS, days=elapsed(MONTHS), days_dtype='<f8'),
                        dict(ok=True, strings=MONTHS, ms=ms_of(MONTHS), ms_source='DerivedFromStrings',
                             hour_2400_rows=[]))
    c['ms_strings_real_form'] = (
        dict(strings=['01JAN1999 12:00:00', '01JAN1999 13:00:00'],
             ms=['01JAN1999 12:00:00:000', '01JAN1999 13:00:00:000'],
             days=[0.0, 1/24], days_dtype='<f4', rows_result=2),
        dict(ok=True, strings=['01JAN1999 12:00:00', '01JAN1999 13:00:00'],
             ms=ms_of(['01JAN1999 12:00:00', '01JAN1999 13:00:00']),
             ms_strings=['01JAN1999 12:00:00:000', '01JAN1999 13:00:00:000'],
             ms_source='StoredStringDataset', hour_2400_rows=[]))
    c['ms_strings_2400'] = (
        dict(strings=ROLL, ms=ms_strings(ROLL), days=elapsed(ROLL), days_dtype='<f4', rows_result=4),
        dict(ok=True, strings=ROLL, ms=ms_of(ROLL), ms_strings=ms_strings(ROLL),
             ms_source='StoredStringDataset', hour_2400_rows=[2]))
    SUB = ['01JAN2020 00:00:00:250', '01JAN2020 00:00:00:750', '01JAN2020 24:00:00:000']
    c['ms_strings_subsecond_only'] = (
        dict(ms=SUB, days=[0.0, 0.5/86400, 86399.75/86400], days_dtype='<f8'),
        dict(ok=True, strings=None, ms=[oracle_ms(x) for x in SUB], ms_strings=SUB,
             ms_source='StoredStringDataset', hour_2400_rows=[2]))
    bad_ms = ms_strings(ROLL)
    bad_ms[1] = '31DEC2019 23:00:01:000'
    SUBS = ['31DEC2019 22:00:00:250', '31DEC2019 23:00:00:999', '31DEC2019 24:00:00:000', '01JAN2020 01:00:00:500']
    c['ms_strings_subsecond_with_stamps'] = (
        dict(strings=ROLL, ms=SUBS, days=[(oracle_ms(x) - oracle_ms(SUBS[0])) / 86400000 for x in SUBS], days_dtype='<f8'),
        dict(ok=True, strings=ROLL, ms=[oracle_ms(x) for x in SUBS], ms_strings=SUBS,
             ms_source='StoredStringDataset', hour_2400_rows=[2]))
    DAILY = [(datetime.date(2000, 1, 1) + datetime.timedelta(days=i)) for i in range(3000)]
    DAILY_S = ['%02d%s%04d 24:00:00' % (d.day, MON[d.month - 1], d.year) for d in DAILY]
    c['daily_2400_many_rows'] = (dict(strings=DAILY_S),
                                 dict(ok=True, strings=DAILY_S, ms=ms_of(DAILY_S), ms_source='DerivedFromStrings',
                                      hour_2400_rows=list(range(3000))))
    c['ms_strings_mismatch'] = (dict(strings=ROLL, ms=bad_ms), dict(error='Corrupt'))
    c['ms_strings_bad_width'] = (dict(ms=['01JAN1999 12:00:00:00']), dict(error='Unsupported'))
    c['ms_strings_bad_value'] = (dict(ms=['29FEB2019 12:00:00:000']), dict(error='Corrupt'))
    c['ms_strings_non_monotonic'] = (dict(ms=['01JAN2020 02:00:00:000', '01JAN2020 01:00:00:000']), dict(error='Corrupt'))
    c['days_disagree_with_stamps'] = (dict(strings=ROLL, days=[0, 2/24, 4/24, 6/24], days_dtype='<f8'), dict(error='Corrupt'))
    c['ms_only'] = (dict(ms=ms_of(ROLL)),
                    dict(ok=True, strings=None, ms=None, ms_source=None, hour_2400_rows=[],
                         raw_ms=ms_of(ROLL), integer_ms_unsupported=True))
    c['days_only'] = (dict(days=[0.0, 0.5, 1.0], days_dtype='<f4'),
                      dict(ok=True, strings=None, ms=None, ms_source=None, hour_2400_rows=[]))
    bad = ms_of(ROLL)
    bad[1] += 1000
    # integer ms has no proven epoch: it is retained, never compared or interpreted
    c['integer_ms_not_interpreted'] = (dict(strings=ROLL, ms=bad),
                                       dict(ok=True, strings=ROLL, ms=ms_of(ROLL), ms_source='DerivedFromStrings',
                                            hour_2400_rows=[2], raw_ms=bad, integer_ms_unsupported=True))
    c['ms_extreme_integers'] = (dict(ms=[-2**63, 0, 2**63 - 1], days=[0.0, 1.0, 2.0], days_dtype='<f8'),
                                dict(ok=True, strings=None, ms=None, ms_source=None, hour_2400_rows=[],
                                     raw_ms=[-2**63, 0, 2**63 - 1], integer_ms_unsupported=True))
    c['ms_u64_overflow'] = (dict(ms=[2**63], ms_dtype='<u8'), dict(error='Corrupt'))
    nm = ['01JAN2020 02:00:00', '01JAN2020 01:00:00', '01JAN2020 03:00:00']
    c['non_monotonic_strings'] = (dict(strings=nm), dict(error='Corrupt'))
    c['non_monotonic_ms'] = (dict(ms=[3000, 2000, 4000]), dict(error='Corrupt'))
    c['duplicate_rows'] = (dict(strings=['01JAN2020 01:00:00', '01JAN2020 01:00:00']), dict(error='Corrupt'))
    c['length_mismatch_datasets'] = (dict(strings=ROLL[:3], ms=ms_of(ROLL)), dict(error='Corrupt'))
    c['length_mismatch_results'] = (dict(strings=ROLL, rows_result=5), dict(error='Corrupt'))
    c['impossible_leap_day'] = (dict(strings=['29FEB2019 01:00:00']), dict(error='Corrupt'))
    c['hour_25'] = (dict(strings=['01JAN2020 25:00:00']), dict(error='Corrupt'))
    c['unparseable_stamp'] = (dict(strings=['2020-01-01T00:00:00']), dict(error='Unsupported'))
    c['bad_month_name'] = (dict(strings=['01XYZ2020 00:00:00']), dict(error='Unsupported'))
    c['float_ms_dataset'] = (dict(ms=[1.0, 2.0], ms_dtype='<f8'), dict(error='Unsupported'))
    c['empty_group'] = (dict(), dict(error='NotFound'))
    c['units_full'] = (dict(units=units_full), dict(units='full'))
    c['units_real_layouts'] = (dict(units=units_real_layouts), dict(units='real'))
    c['units_priority_edges'] = (dict(units=units_priority_edges), dict(units='edges'))
    c['units_latin1_numeric'] = (dict(units=units_latin1), dict(units='latin1'))
    c['units_missing'] = (dict(units=units_missing), dict(units='missing'))
    return c

def main(out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    manifest = {'generator': 'make_fixtures.py', 'epoch': 'civil 1970-01-01T00:00:00 (no zone)',
                'fixtures': {}}
    for name, (kwargs, expect) in cases().items():
        p = out / (name + '.h5')
        make(p, **kwargs)
        manifest['fixtures'][name] = {'file': p.name, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest(),
                                      'expect': expect}
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(len(manifest['fixtures']), 'fixtures')

if __name__ == '__main__':
    main(sys.argv[1])
