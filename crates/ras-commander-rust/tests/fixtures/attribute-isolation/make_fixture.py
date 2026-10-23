"""Freeze attribute-isolation reproducers; Python -I, h5py/numpy only.
Usage: make_fixture.py output.h5 [projection]
"""
from pathlib import Path
import h5py, numpy as np, sys, hashlib
p = Path(sys.argv[1])
projection = len(sys.argv) > 2 and sys.argv[2] == 'projection'
with h5py.File(p, 'w', libver='earliest') as f:
    if projection:
        f.attrs.create('Projection', np.array([(b'a', 7)], dtype=[('Name', 'S4'), ('Count', '<i4')]))
        envelope = f.attrs.get_id('Projection').get_type().encode()[2:]
    else:
        f.attrs.create('bad', np.array([(b'a', 7)], dtype=[('Name', 'S4'), ('Count', '<i4')]))
        envelope = f.attrs.get_id('bad').get_type().encode()[2:]
        f.attrs.create('Projection', np.bytes_('PROJCS["x"]'))
    f.attrs.create('Units System', np.int32(1))
b = bytearray(p.read_bytes())
at = b.index(envelope)
if projection:
    b[at+16:at+20] = (99).to_bytes(4, 'little')
else:
    b[at+4:at+8] = bytes(4)
p.write_bytes(b)
print(hashlib.sha256(b).hexdigest())
