"""Root `File Type` placements that a prefix of the file does not hold (tests/file_type_bytes.rs).

    python tests/generate_file_type_fixtures.py --output tests/fixtures/file_type
"""
from pathlib import Path
import argparse, hashlib, json
import h5py, numpy as np


def text(obj, name, value):
    obj.attrs.create(name, np.bytes_(value), dtype=h5py.string_dtype('ascii', len(value)))


def userblock(path):
    """A 128 KiB user block: the superblock and every object are past the first 64 KiB."""
    with h5py.File(path, 'w', userblock_size=128 * 1024) as f:
        text(f, 'File Type', b'HEC-RAS Results')
        f.create_dataset('Geometry/x', data=np.arange(4, dtype='<f8'))


def late_dense_attributes(path):
    """Root attributes in dense storage, written after 80 KiB of data: the attribute heap
    and its B-tree lie past the first 64 KiB."""
    with h5py.File(path, 'w', libver='latest') as f:
        f.create_dataset('Geometry/big', data=np.zeros(10 * 1024, dtype='<f8'))
        for i in range(40):
            text(f, f'Note {i:02d}', b'x' * 64)
        text(f, 'File Type', b'HEC-RAS Geometry')


def late_dense_absent(path):
    """As above, with no File Type among the attributes."""
    with h5py.File(path, 'w', libver='latest') as f:
        f.create_dataset('Geometry/big', data=np.zeros(10 * 1024, dtype='<f8'))
        for i in range(40):
            text(f, f'Note {i:02d}', b'x' * 64)


CASES = dict(userblock=userblock, late_dense_attributes=late_dense_attributes,
             late_dense_absent=late_dense_absent)

if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    for name, build in CASES.items():
        build(a.output / f'{name}.h5')
    (a.output / 'provenance.json').write_bytes((json.dumps(dict(
        source='synthetic h5py %s (tests/generate_file_type_fixtures.py)' % h5py.__version__,
        files={q.name: hashlib.sha256(q.read_bytes()).hexdigest() for q in sorted(a.output.glob('*.h5'))}),
        indent=2) + '\n').encode())
