"""Synthetic RASMapper terrain-modification fixtures (TerrainModifications read family).

The files reproduce the stored schema observed in real HEC-RAS 6.1-6.6 derived terrain HDFs
and in ras-commander `RasTerrainModWriter` output (datatypes, member names/order, packed
offsets, NULLTERM/NULLPAD ASCII text, little-endian f4, unsigned bytes, chunked+deflate
storage, group Type/Subtype/Priority attributes). Every name, coordinate and value below is
invented; no real model data is copied.

- valid.h5: one group per observed layout (polyline Channel and Levee subtypes in the 8-member
  and the 10-member System Name/Transition Percent layouts, the writer's Type=Channel and
  Profile/empty Control Points variant, Polygon with four-column points and Control Points,
  Circle, Pier, Rectangle), plus a SPACEPAD/NULLPAD text variant.
- refusals.h5: one valid group next to one group per refused defect (wrong widths, byte
  order, signedness, vlen/UTF-8 text, order, missing member, padding, Type, row count, rank,
  and near misses of the 10-member, Pier and Rectangle layouts).
- truncated.h5: a valid table whose contiguous payload is cut off at end of file.

Output is byte-reproducible for a given h5py/HDF5 build: object modification times are not
recorded.

The manifest records, per Attributes table, the expected engine status and, for qualified
tables, the exact stored compound bytes and the bytes h5py returns for `dataset[()]`.
Usage: python generate_terrain_modification_fixtures.py --output tests/fixtures/terrain_modifications
"""
from pathlib import Path
import argparse, hashlib, json
import h5py, numpy as np

p = argparse.ArgumentParser()
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=True)

NT, NP, SP = h5py.h5t.STR_NULLTERM, h5py.h5t.STR_NULLPAD, h5py.h5t.STR_SPACEPAD


def text(width, pad=NT, utf8=False):
    t = h5py.h5t.C_S1.copy()
    t.set_size(width)
    t.set_strpad(pad)
    if utf8:
        t.set_cset(h5py.h5t.CSET_UTF8)
    return t


F4, F8, F4BE = h5py.h5t.IEEE_F32LE, h5py.h5t.IEEE_F64LE, h5py.h5t.IEEE_F32BE
U1, I1 = h5py.h5t.STD_U8LE, h5py.h5t.STD_I8LE
VLEN = h5py.h5t.special_dtype(vlen=bytes)


def compound(members, size=None):
    """Packed compound (or with `size` > packed width, trailing padding)."""
    widths = [m[1].get_size() for m in members]
    t = h5py.h5t.create(h5py.h5t.COMPOUND, size or sum(widths))
    at = 0
    for (name, mt), w in zip(members, widths):
        t.insert(name.encode(), at, mt)
        at += w
    return t


def raw_rows(members, rows, size=None):
    """Exact stored bytes for rows of python values (bytes for text, numbers otherwise)."""
    out = bytearray()
    for row in rows:
        rec = bytearray()
        for (name, mt), v in zip(members, row):
            w = mt.get_size()
            if mt.get_class() == h5py.h5t.STRING:
                assert len(v) <= w
                rec += v + b'\0' * (w - len(v))
            elif mt.get_class() == h5py.h5t.FLOAT:
                rec += np.array([v], dtype=('>' if mt.get_order() else '<') + f'f{w}').tobytes()
            else:
                rec += np.array([v], dtype=('<i1' if mt.get_sign() else '<u1')).tobytes()
        rec += b'\0' * ((size or len(rec)) - len(rec))
        out += rec
    return bytes(out)


def table(group, name, members, rows, size=None, chunked=True, shape=None):
    t = compound(members, size)
    n = len(rows)
    shape = shape or (n,)
    plist = h5py.h5p.create(h5py.h5p.DATASET_CREATE)
    plist.set_obj_track_times(False)
    if chunked:
        plist.set_chunk((max(n, 1),) + shape[1:])
        plist.set_deflate(1)
    space = h5py.h5s.create_simple(shape)
    d = h5py.h5d.create(group.id, name.encode(), t, space, dcpl=plist)
    data = np.frombuffer(raw_rows(members, rows, size), dtype=np.uint8)
    d.write(h5py.h5s.ALL, h5py.h5s.ALL, data, mtype=t)
    return d


def vlen_table(group, name, rows):
    dt = np.dtype([('Name', VLEN), ('Elevation Value', '<f4'), ('Elevation Type', 'S11')])
    group.create_dataset(name, data=np.array(rows, dtype=dt), chunks=True, compression='gzip',
                         track_times=False)


def sattr(obj, name, value, pad=NT):
    t = text(len(value), pad)
    a = h5py.h5a.create(obj.id, name.encode(), t, h5py.h5s.create(h5py.h5s.SCALAR))
    a.write(np.array(value, dtype=f'S{len(value)}'), mtype=t)


def group(root, name, kind, subtype=None, priority=0):
    g = root.create_group(name)
    sattr(g, 'Type', kind.encode())
    if subtype:
        sattr(g, 'Subtype', subtype.encode())
    g.attrs.create('Priority', priority, dtype='<i4')
    return g


def arr(g, name, values, dtype):
    g.create_dataset(name, data=np.asarray(values, dtype=dtype), chunks=True, compression='gzip',
                     track_times=False)


def polyline_arrays(g, n, columns=2):
    arr(g, 'Polyline Info', [[2 * i, 2, i, 1] for i in range(n)], '<i4')
    arr(g, 'Polyline Parts', [[0, 2]] * n, '<i4')
    arr(g, 'Polyline Points', [[100.0 * j + i for j in range(1, columns + 1)] for i in range(2 * n)],
        '<f8')
    arr(g, 'Profile Info', [[0, 0]] * n, '<i4')
    g.create_dataset('Profile Values', shape=(0, 2), dtype='<f4', maxshape=(None, 2), chunks=(1, 2),
                     track_times=False)


def polygon_arrays(g, n, columns=2):
    arr(g, 'Polygon Info', [[4 * i, 4, i, 1] for i in range(n)], '<i4')
    arr(g, 'Polygon Parts', [[0, 4]] * n, '<i4')
    arr(g, 'Polygon Points', [[10.0 * i + j for j in range(columns)] for i in range(4 * n)], '<f8')


POLYLINE = lambda w, et=11: [('Name', text(w)), ('Elevation Type', text(et)), ('Top Elevation', F4),
                             ('Top Width', F4), ('Left Slope', F4), ('Right Slope', F4),
                             ('Max Reach', F4), ('Elev Pt Tolerance', F4)]
POLYGON = lambda w, pad=NT: [('Name', text(w, pad)), ('Elevation Value', F4),
                             ('Elevation Type', text(11, pad)), ('Elev Pt Tolerance', F4),
                             ('Generate Boundary Elevations', U1), ('Use ShapeFile Z Elevations', U1)]
CIRCLE = lambda w: [('Name', text(w)), ('Elevation Value', F4), ('Elevation Type', text(11))]
# HEC-RAS 6.4.1-6.6 polyline layout; ras-commander's writer stores it NULLPAD with S128/S128/S16.
TRANSITION = lambda sw, w, et=11, pad=NT: [
    ('System Name', text(sw, pad)), ('Name', text(w, pad)), ('Elevation Type', text(et, pad)),
    ('Top Elevation', F4), ('Top Width', F4), ('Left Slope', F4), ('Right Slope', F4),
    ('Max Reach', F4), ('Transition Percent', F4), ('Elev Pt Tolerance', F4)]
TR_ROW = (b'', b'Bad', b'Interpolate', 1, 2, 3, 4, 5, 0.5, 6)
CONTROL = [('Name', text(1)), ('Elevation', F4)]
PL_ROWS = [(b'Cut A', b'Interpolate', 12.5, 30.0, 3.0, 3.0, 50.0, 0.25),
           # NULLTERM text with stored bytes after the terminator: h5py cuts at the first NUL.
           (b'Cut\0Z', b'Set Value\0q', -1.5, 0.0, 2.0, 4.0, 0.0, 0.5)]


def valid(path):
    with h5py.File(path, 'w') as f:
        root = f.create_group('Modifications')
        g = group(root, 'Synthetic Channel', 'Levee', 'Channel', 0)
        table(g, 'Attributes', POLYLINE(16), PL_ROWS)
        polyline_arrays(g, 2)
        g = group(root, 'Synthetic Levee', 'Levee', 'Levee', 1)
        table(g, 'Attributes', POLYLINE(9), [(b'Levee One', b'Interpolate', 101.25, 8.0, 3.0, 3.0, 25.0, 0.0)])
        polyline_arrays(g, 1)
        cp = g.create_group('Control Points')
        table(cp, 'Attributes', CONTROL, [(b'', 99.0), (b'a', 98.5), (b'', float('nan'))])
        arr(cp, 'Points', [[1.0, 2.0, 99.0], [3.0, 4.0, 98.5], [5.0, 6.0, -1.0]], '<f8')
        g = group(root, 'Synthetic Polygon', 'Polygon', None, 2)
        table(g, 'Attributes', POLYGON(10), [(b'Pad One', 55.0, b'Set Value', 0.1, 1, 0),
                                              (b'Pad Two', -3.25, b'Interpolate', 0.0, 0, 1)])
        polygon_arrays(g, 2, columns=4)
        cp = g.create_group('Control Points')
        table(cp, 'Attributes', CONTROL, [(b'', 7.0)])
        arr(cp, 'Points', [[1.0, 1.0, 7.0]], '<f8')
        g = group(root, 'Synthetic Circle', 'Circle', None, 3)
        table(g, 'Attributes', CIRCLE(6), [(b'Ring A', 12.0, b'Set Value'), (b'Ring B', 13.0, b'Elevation')])
        arr(g, 'Circles', [[0.0, 0.0, 5.0, 1.0, 2.0], [9.0, 9.0, 1.0, 1.0, 2.0]], '<f8')
        g = group(root, 'Synthetic Spacepad', 'Polygon', None, 4)
        table(g, 'Attributes', POLYGON(8, SP), [(b'Edge  ', 1.0, b'Set Value  ', 0.0, 0, 0)])
        polygon_arrays(g, 1)
        g = group(root, 'Synthetic Nullpad', 'Polygon', None, 5)
        table(g, 'Attributes', POLYGON(8, NP), [(b'Edge\0x', 2.0, b'Set Value', 0.0, 1, 1)])
        polygon_arrays(g, 1)
        # 10-member transition layout as RASMapper stores it: empty one-byte System Name.
        g = group(root, 'Synthetic Transition Channel', 'Levee', 'Channel', 6)
        table(g, 'Attributes', TRANSITION(1, 12), [
            (b'', b'Ditch North', b'Interpolate', -2.0, 15.0, 3.0, 3.0, 40.0, 0.5, 0.0),
            (b'', b'Ditch\0South', b'Set Value\0z', 4.5, 0.0, 2.0, 2.5, 0.0, 0.25, 1.0)])
        polyline_arrays(g, 2, columns=3)
        g = group(root, 'Synthetic Transition Levee', 'Levee', 'Levee', 7)
        table(g, 'Attributes', TRANSITION(1, 7), [(b'', b'Berm 01', b'TakeHigher', 21.0, 12.0, 3.0, 4.0,
                                                    60.0, 0.75, 0.5)])
        polyline_arrays(g, 1, columns=4)
        cp = g.create_group('Control Points')
        table(cp, 'Attributes', CONTROL, [(b'', 20.0), (b'', 20.5)])
        arr(cp, 'Points', [[1.0, 2.0, 0.0, 20.0], [3.0, 4.0, 1.0, 20.5]], '<f8')
        # ras-commander RasTerrainModWriter output: NULLPAD S128/S128/S16, a Profile array and an
        # empty typed Control Points group; Type=Channel is the writer's own spelling.
        for name, kind in [('Synthetic Writer Levee', 'Levee'), ('Synthetic Writer Channel', 'Channel')]:
            g = group(root, name, kind, kind, 0)
            table(g, 'Attributes', TRANSITION(128, 128, 16, NP), [
                (name.encode(), name.encode(), b'TakeHigher' if kind == 'Levee' else b'TakeLower',
                 30.0, 20.0, 2.0, 2.0, 100.0, 0.5, 50.0)])
            polyline_arrays(g, 1)
            arr(g, 'Profile', [[0.0, 30.0], [10.0, 30.0]], '<f8')
            cp = g.create_group('Control Points')
            sattr(cp, 'Type', b'ControlCrossSection')
        g = group(root, 'Synthetic Pier', 'Pier', None, 8)
        table(g, 'Attributes', CIRCLE(1), [(b'', 3.5, b'Set Value'), (b'', 4.0, b'TakeHigher')])
        arr(g, 'Piers', [[float(i + j) for j in range(15)] for i in range(2)], '<f8')
        g = group(root, 'Synthetic Rectangle', 'Rectangle', None, 9)
        table(g, 'Attributes', CIRCLE(14), [(b'Culvert Box 01', 7.25, b'TakeLower')])
        arr(g, 'Rectangles', [[0.0, 0.0, 4.0, 2.0, 0.5]], '<f8')


def refusals(path):
    with h5py.File(path, 'w') as f:
        root = f.create_group('Modifications')
        g = group(root, 'Valid Channel', 'Levee', 'Channel')
        table(g, 'Attributes', POLYLINE(8), [(b'Okay', b'Interpolate', 1, 2, 3, 4, 5, 6)])
        polyline_arrays(g, 1)

        def pl(name, members, rows=None, n=1, kind='Levee', size=None, shape=None, arrays=True):
            g = group(root, name, kind, 'Channel' if kind == 'Levee' else None)
            table(g, 'Attributes', members, rows or [(b'Bad', b'Interpolate', 1, 2, 3, 4, 5, 6)] * n,
                  size=size, shape=shape)
            if arrays:
                polyline_arrays(g, 1)
            return g

        wide = POLYLINE(8)
        wide[2] = ('Top Elevation', F8)
        pl('Wide Float', wide)
        big = POLYLINE(8)
        big[3] = ('Top Width', F4BE)
        pl('Big Endian', big)
        pl('Missing Member', POLYLINE(8)[:6] + POLYLINE(8)[7:], [(b'Bad', b'Interpolate', 1, 2, 3, 4, 6)])
        swapped = POLYLINE(8)
        swapped[2], swapped[3] = swapped[3], swapped[2]
        pl('Reordered', swapped)
        pl('Padded', POLYLINE(8), size=48)
        pl('Unknown Type', POLYLINE(8), kind='Spline')
        pl('Row Mismatch', POLYLINE(8), n=2)
        pl('Rank Two', POLYLINE(8), shape=(1, 1))
        utf = POLYLINE(8)
        utf[0] = ('Name', text(8, utf8=True))
        pl('Utf8 Name', utf)
        g = group(root, 'Type Mismatch', 'Circle')
        table(g, 'Attributes', POLYGON(8), [(b'Bad', 1.0, b'Set Value', 0.0, 0, 0)])
        arr(g, 'Circles', [[0.0, 0.0, 1.0, 1.0, 1.0]], '<f8')
        signed = POLYGON(8)
        signed[4] = ('Generate Boundary Elevations', I1)
        g = group(root, 'Signed Byte', 'Polygon')
        table(g, 'Attributes', signed, [(b'Bad', 1.0, b'Set Value', 0.0, 0, 0)])
        polygon_arrays(g, 1)
        g = group(root, 'Missing Feature', 'Polygon')
        table(g, 'Attributes', POLYGON(8), [(b'Bad', 1.0, b'Set Value', 0.0, 0, 0)])
        g = root.create_group('No Type')
        table(g, 'Attributes', POLYLINE(8), [(b'Bad', b'Interpolate', 1, 2, 3, 4, 5, 6)])
        polyline_arrays(g, 1)
        g = group(root, 'Vlen Name', 'Circle')
        vlen_table(g, 'Attributes', [(b'Bad', 1.0, b'Set Value')])
        arr(g, 'Circles', [[0.0, 0.0, 1.0, 1.0, 1.0]], '<f8')
        # Near misses of the 10-member, Pier and Rectangle layouts.
        pl('Transition Missing Percent', TRANSITION(1, 8)[:8] + TRANSITION(1, 8)[9:],
           [TR_ROW[:8] + TR_ROW[9:]])
        swapped = TRANSITION(1, 8)
        swapped[8], swapped[9] = swapped[9], swapped[8]
        pl('Transition Reordered', swapped, [TR_ROW])
        renamed = TRANSITION(1, 8)
        renamed[0] = ('Sys Name', text(1))
        pl('Transition Renamed', renamed, [TR_ROW])
        wide = TRANSITION(1, 8)
        wide[8] = ('Transition Percent', F8)
        pl('Transition Wide', wide, [TR_ROW])
        pl('Transition Padded', TRANSITION(1, 8), [TR_ROW], size=64)
        pl('Transition Row Mismatch', TRANSITION(1, 8), [TR_ROW] * 2)
        # Type=Channel is qualified only in the transition layout it was observed with.
        pl('Channel Type Short Layout', POLYLINE(8), kind='Channel')
        g = group(root, 'Pier Missing Feature', 'Pier')
        table(g, 'Attributes', CIRCLE(8), [(b'Bad', 1.0, b'Set Value')])
        arr(g, 'Circles', [[0.0, 0.0, 1.0, 1.0, 1.0]], '<f8')
        g = group(root, 'Pier Wide', 'Pier')
        wide = CIRCLE(8)
        wide[1] = ('Elevation Value', F8)
        table(g, 'Attributes', wide, [(b'Bad', 1.0, b'Set Value')])
        arr(g, 'Piers', [[0.0] * 15], '<f8')
        g = group(root, 'Rectangle Row Mismatch', 'Rectangle')
        table(g, 'Attributes', CIRCLE(8), [(b'Bad', 1.0, b'Set Value')] * 2)
        arr(g, 'Rectangles', [[0.0, 0.0, 1.0, 1.0, 1.0]], '<f8')
        g = group(root, 'Rectangle Polygon Layout', 'Rectangle')
        table(g, 'Attributes', POLYGON(8), [(b'Bad', 1.0, b'Set Value', 0.0, 0, 0)])
        arr(g, 'Rectangles', [[0.0, 0.0, 1.0, 1.0, 1.0]], '<f8')


def truncated(path):
    with h5py.File(path, 'w', libver='earliest') as f:
        root = f.create_group('Modifications')
        g = group(root, 'Cut Short', 'Circle')
        arr(g, 'Circles', [[0.0, 0.0, 1.0, 1.0, 1.0]] * 64, '<f8')
        table(g, 'Attributes', CIRCLE(24), [(b'Row %02d' % i, float(i), b'Set Value') for i in range(64)],
              chunked=False)
        off = g['Attributes'].id.get_offset()
        size = g['Attributes'].id.get_storage_size()
    data = path.read_bytes()
    assert off + size == len(data), (off, size, len(data))
    path.write_bytes(data[:off + size // 2])


def stored_bytes(ds):
    """The dataset's stored compound bytes, read with the file type as memory type."""
    t = ds.id.get_type()
    out = np.empty(ds.shape, dtype=np.dtype((np.void, t.get_size())))
    ds.id.read(h5py.h5s.ALL, h5py.h5s.ALL, out, mtype=t)
    return out.tobytes()


EXPECT = {
    'valid.h5': {},
    'refusals.h5': {
        'Wide Float': ('unsupported', 'schema'), 'Big Endian': ('unsupported', 'schema'),
        'Missing Member': ('unsupported', 'schema'), 'Reordered': ('unsupported', 'schema'),
        'Padded': ('unsupported', 'schema'), 'Unknown Type': ('unsupported', 'schema'),
        'Row Mismatch': ('corrupt', None), 'Rank Two': ('unsupported', 'schema'),
        'Utf8 Name': ('unsupported', 'schema'), 'Type Mismatch': ('unsupported', 'schema'),
        'Signed Byte': ('unsupported', 'schema'), 'Missing Feature': ('unsupported', 'schema'),
        'No Type': ('unsupported', 'schema'), 'Vlen Name': ('unsupported', 'encoding'),
        'Transition Missing Percent': ('unsupported', 'schema'),
        'Transition Reordered': ('unsupported', 'schema'),
        'Transition Renamed': ('unsupported', 'schema'),
        'Transition Wide': ('unsupported', 'schema'),
        'Transition Padded': ('unsupported', 'schema'),
        'Transition Row Mismatch': ('corrupt', None),
        'Channel Type Short Layout': ('unsupported', 'schema'),
        'Pier Missing Feature': ('unsupported', 'schema'), 'Pier Wide': ('unsupported', 'schema'),
        'Rectangle Row Mismatch': ('corrupt', None),
        'Rectangle Polygon Layout': ('unsupported', 'schema'),
    },
}
ROLES = {'Levee': 'polyline_modification_attributes', 'Channel': 'polyline_modification_attributes',
         'Polygon': 'polygon_modification_attributes', 'Circle': 'circle_modification_attributes',
         'Pier': 'pier_modification_attributes', 'Rectangle': 'rectangle_modification_attributes'}
FEATURES = {'Levee': 'Polyline Info', 'Channel': 'Polyline Info', 'Polygon': 'Polygon Info',
            'Circle': 'Circles', 'Pier': 'Piers', 'Rectangle': 'Rectangles'}

manifest = []
for name, build in [('valid.h5', valid), ('refusals.h5', refusals), ('truncated.h5', truncated)]:
    q = a.output / name
    build(q)
    entry = {'name': name, 'sha256': hashlib.sha256(q.read_bytes()).hexdigest(), 'tables': []}
    if name == 'truncated.h5':
        try:
            with h5py.File(q, 'r') as f:
                f['Modifications/Cut Short/Attributes'][()]
            entry['h5py'] = 'read'
        except Exception as e:
            entry['h5py'] = type(e).__name__
        entry['tables'].append({'path': 'Modifications/Cut Short/Attributes', 'status': 'refused'})
        manifest.append(entry)
        continue
    with h5py.File(q, 'r') as f:
        for gname, g in f['Modifications'].items():
            for sub, ds in [('', g['Attributes'])] + ([('Control Points/', g['Control Points/Attributes'])]
                                                      if 'Control Points/Attributes' in g else []):
                path = f'Modifications/{gname}/{sub}Attributes'
                status, code = EXPECT[name].get(gname, ('supported', None))
                row = {'path': path, 'status': status, 'code': code}
                if status == 'supported':
                    kind = g.attrs['Type'].decode()
                    row['role'] = 'control_point_attributes' if sub else ROLES[kind]
                    row['identity_table'] = (f'Modifications/{gname}/{sub}'
                                             + ('Points' if sub else FEATURES[kind]))
                    row['rows'] = int(ds.shape[0])
                    row['columns'] = [[n, int(ds.dtype.fields[n][1])] for n in ds.dtype.names]
                    row['stored_hex'] = stored_bytes(ds).hex()
                    row['h5py_hex'] = ds[()].tobytes().hex()
                    row['h5py_dtype'] = str(ds.dtype.descr)
                entry['tables'].append(row)
    manifest.append(entry)
(a.output / 'manifest.json').write_text(json.dumps(manifest, indent=1) + '\n')
