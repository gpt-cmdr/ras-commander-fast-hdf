"""Generate synthetic full-width H5T_BITFIELD datasets, not integer datasets."""
from pathlib import Path
import hashlib, json
import h5py
import numpy as np

root = Path(__file__).parent / 'fixtures' / 'bitfield_numeric'
root.mkdir(parents=True, exist_ok=True)
with h5py.File(root / 'bitfields.h5', 'w', libver='earliest') as f:
    for width in (1, 2, 4, 8):
        for endian in ('LE', 'BE'):
            t = getattr(h5py.h5t, f'STD_B{width * 8}{endian}').copy()
            dtype = np.dtype(('<' if endian == 'LE' else '>') + f'u{width}')
            v = np.array([0, 1, 2, 3, (1 << (width*8-1)), (1 << (width*8))-1], dtype=dtype).reshape(2,3)
            for layout in ('contiguous', 'chunked'):
                props = h5py.h5p.create(h5py.h5p.DATASET_CREATE)
                if layout == 'chunked':
                    props.set_chunk((1, 2)); props.set_shuffle(); props.set_deflate(4)
                d = h5py.h5d.create(f.id, f'b{width*8}_{endian}_{layout}'.encode(), t,
                    h5py.h5s.create_simple(v.shape), dcpl=props)
                d.write(h5py.h5s.ALL, h5py.h5s.ALL, v, mtype=t)
    f.copy('b8_LE_chunked', f.require_group('Terrain/test/0'), name='Mask')
    for name, precision, offset in [('partial', 7, 0), ('offset', 7, 1)]:
        base = h5py.h5t.STD_B8LE.copy()
        # h5py exposes atomic setters on TypeIntegerID; HDF5 accepts bitfields.
        t = h5py.h5t.TypeIntegerID(base.id)
        h5py.h5i.inc_ref(t)  # both wrappers own their reference
        t.set_precision(precision); t.set_offset(offset)
        d = h5py.h5d.create(f.id, name.encode(), t, h5py.h5s.create_simple((3,)))
        d.write(h5py.h5s.ALL, h5py.h5s.ALL, np.array([0, 1, 127], dtype='u1'), mtype=t)
(root/'manifest.json').write_text(json.dumps({'source': 'Synthetic H5T_BITFIELD datasets generated with h5py',
    'sha256': hashlib.sha256((root/'bitfields.h5').read_bytes()).hexdigest()}, indent=2)+'\n')
