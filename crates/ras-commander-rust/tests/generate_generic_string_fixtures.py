"""Synthetic generic-string boundary fixtures (H4, review MPR01/MPR02 residual).

Each file is a tiny land-cover-like parallel-array table (IDs, ManningsN, Names) whose
`Names` dataset exercises one text-encoding boundary of the generic dataset string
reader. Corruptions are byte patches of an h5py-written file: a vlen reference's
global-heap object index (missing object; 65537, which a u16 narrowing aliases to
object 1) and the class-3 datatype flag byte (reserved padding 3; reserved charset 2).
h5py 3.14/HDF5 refuses every corrupt case; the engine must refuse them too.
Usage: python generate_generic_string_fixtures.py --output tests/fixtures/generic_strings
"""
from pathlib import Path
import argparse, hashlib, json, struct
import h5py, numpy as np

p = argparse.ArgumentParser()
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=True)
WIDTH = 13


def base(path, writer):
    with h5py.File(path, 'w', libver='earliest') as f:
        f.create_dataset('IDs', data=np.array([1, 2, 3], dtype='<i4'))
        f.create_dataset('ManningsN', data=np.array([0.03, 0.05, 0.08], dtype='<f4'))
        writer(f)


def vlen(f):
    f.create_dataset('Names', data=np.array([b'Grass', b'Forest', b'Urban'], dtype=object),
                     dtype=h5py.string_dtype('ascii'))


def fixed(f):
    tid = h5py.h5t.C_S1.copy()
    tid.set_size(WIDTH)
    tid.set_strpad(h5py.h5t.STR_NULLTERM)
    d = h5py.h5d.create(f.id, b'Names', tid, h5py.h5s.create_simple((3,)))
    d.write(h5py.h5s.ALL, h5py.h5s.ALL, np.array([b'Grass\x00X', b'Forest', b'Urban'], dtype=f'S{WIDTH}'))


def heap_index(path, row, index):
    with h5py.File(path, 'r') as f:
        off = f['Names'].id.get_offset()
    with open(path, 'r+b') as fh:
        fh.seek(off + row * 16 + 12)  # u32 length, u64 heap address, u32 object index
        fh.write(struct.pack('<I', index))


def flags(path, value):
    data = bytearray(path.read_bytes())
    sig = bytes([0x13])
    hits = [i for i in range(len(data) - 8) if data[i:i + 1] == sig and data[i + 2:i + 4] == b'\0\0'
            and data[i + 4:i + 8] == struct.pack('<I', WIDTH)]
    assert len(hits) == 1, hits
    data[hits[0] + 1] = value
    path.write_bytes(bytes(data))


CASES = {
    'vlen_valid.h5': (vlen, None, 'unsupported'),
    'vlen_missing_object.h5': (vlen, lambda q: heap_index(q, 1, 9), 'unsupported'),
    'vlen_high_index.h5': (vlen, lambda q: heap_index(q, 1, 0x10001), 'unsupported'),
    'fixed_nullterm.h5': (fixed, None, 'text'),
    'fixed_reserved_padding.h5': (fixed, lambda q: flags(q, 0x03), 'unsupported'),
    'fixed_reserved_charset.h5': (fixed, lambda q: flags(q, 0x20), 'unsupported'),
}
manifest = []
for name, (writer, patch, expected) in CASES.items():
    q = a.output / name
    base(q, writer)
    if patch:
        patch(q)
    try:
        with h5py.File(q, 'r') as f:
            h5 = [x.decode() if isinstance(x, bytes) else x for x in f['Names'][()]]
    except Exception as e:
        h5 = f'{type(e).__name__}'
    manifest.append({'name': name, 'expected': expected, 'h5py': h5,
                     'sha256': hashlib.sha256(q.read_bytes()).hexdigest()})
(a.output / 'manifest.json').write_text(json.dumps(manifest, indent=1) + '\n')
