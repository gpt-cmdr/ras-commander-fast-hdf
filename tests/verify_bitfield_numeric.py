"""Compare native bitfield full/batch/session/slab reads to h5py.

Run against an installed development wheel. RAS_BITFIELD_TERRAIN optionally
selects a public HEC-RAS terrain HDF for the real-format mask check.
"""
from pathlib import Path
import os
import sys
import h5py
import numpy as np
import ras_commander_fast_hdf as native

fixture = Path(__file__).parent / 'fixtures/bitfield_numeric/bitfields.h5'
checks = 0

def verify(path, names):
    global checks
    with h5py.File(path, 'r') as h, native.HdfSession(path, threads=2) as session:
        for name in names:
            expected = h[name][...]
            def exact(actual, expected=expected):
                global checks
                assert actual.dtype == expected.dtype, (name, actual.dtype, expected.dtype)
                assert actual.shape == expected.shape
                assert actual.tobytes() == expected.tobytes(), name
                checks += 1
            exact(native.read_numeric(path, name))
            exact(native.read_numeric_batch(path, [name])[name])
            exact(session.read_numeric(name))
            exact(session.read_numeric_batch([name])[name])
            selection = [(0, n, 2) for n in expected.shape]
            exact(session.read_numeric(name, selection), expected[tuple(slice(0,n,2) for n in expected.shape)])
            exact(native.read_numeric(path, name, selection), expected[tuple(slice(0,n,2) for n in expected.shape)])

with h5py.File(fixture, 'r') as h:
    names = [name for name in h if name.startswith('b') and
        ('_LE_' in name) == (sys.byteorder == 'little')]
verify(fixture, names)
if os.environ.get('RAS_BITFIELD_TERRAIN'):
    path = Path(os.environ['RAS_BITFIELD_TERRAIN'])
    with h5py.File(path, 'r') as h:
        names = []
        h.visititems(lambda name, obj: names.append(name) if isinstance(obj, h5py.Dataset)
            and name.endswith('/Mask') else None)
        assert names, 'No terrain masks found'
        for name in names:
            t = h[name].id.get_type()
            assert t.get_class() == h5py.h5t.BITFIELD
    verify(path, names)
print(f'{checks} exact h5py comparisons passed; extension API {native.API_VERSION}')
