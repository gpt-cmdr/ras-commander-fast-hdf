"""Synthetic 2D mesh corner-elevation fixtures (MeshCornerElevations read family).

The files reproduce the stored schema observed in real HEC-RAS 7.0 and 7.0.1 geometry and
plan HDFs: `Geometry/2D Flow Areas/<mesh>/FacePoints Elevation` is a rank-one little-endian
f4 array with one value per `FacePoints Coordinate` row, chunked with deflate, and carries a
fixed-length ASCII `Can Plot = False` attribute. Every name, coordinate and value below is
invented; no real model data is copied.

- geometry.h5: the geometry layout. `Mesh A` (chunked+deflate) stores NaN with a payload,
  negative zero, infinities, a subnormal and the -9999 sentinel; `Mesh C` is contiguous;
  `Mesh B` has coordinates but no corner elevations (absent).
- cache.h5: the RASMapper mapping-cache layout only (`.../Processed Data/Profile
  (Horizontal)/FacePoint Elevation`), as in a `PostProcessing.hdf` without geometry.
- refusals.h5: one mesh per refused defect (length, f8, big-endian f4, i4, rank two,
  scalar, missing coordinates, coordinates of the wrong width).

Output is byte-reproducible for a given h5py/HDF5 build: object modification times are not
recorded. The manifest records, per dataset, the expected engine status and, for qualified
datasets, the exact stored bytes and the bytes h5py returns for `dataset[()]`.
Usage: python generate_corner_elevation_fixtures.py --output tests/fixtures/corner_elevations
"""
from pathlib import Path
import argparse, hashlib, json
import h5py, numpy as np

p = argparse.ArgumentParser()
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=True)

GEOMETRY = 'Geometry/2D Flow Areas'
CACHE = 'Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas'
CACHE_DATASET = 'Processed Data/Profile (Horizontal)/FacePoint Elevation'


def bits(values):
    """f4 values from exact little-endian bit patterns (so NaN payloads survive)."""
    return np.array(values, dtype='<u4').view('<f4')


# NaN with payload 1, -0.0, +inf, -inf, smallest subnormal, -9999 sentinel, ordinary values.
SPECIAL = bits([0x7FC00001, 0x80000000, 0x7F800000, 0xFF800000, 0x00000001,
                0xC61C3C00, 0x42C80000, 0x447A0000])
PLAIN = np.array([10.5, 11.25, 12.0], dtype='<f4')


def fixed_text(value):
    t = h5py.h5t.C_S1.copy()
    t.set_size(len(value))
    t.set_strpad(h5py.h5t.STR_NULLTERM)
    return t


def can_plot(ds):
    space = h5py.h5s.create(h5py.h5s.SCALAR)
    attr = h5py.h5a.create(ds.id, b'Can Plot', fixed_text('False'), space)
    attr.write(np.array(b'False', dtype='S5'))


def coords(g, n, width=2):
    g.create_dataset('FacePoints Coordinate', data=np.arange(n * width, dtype='<f8').reshape(n, width),
                     track_times=False)


def elevation(g, name, data, dtype='<f4', chunked=True, shape=None):
    data = np.asarray(data).astype(dtype)
    if shape is not None:
        data = data.reshape(shape)
    kw = dict(chunks=data.shape, compression='gzip', compression_opts=1) if chunked and data.ndim else {}
    ds = g.create_dataset(name, data=data, track_times=False, **kw)
    can_plot(ds)
    return ds


def new_file(path):
    f = h5py.File(path, 'w', libver='earliest', track_order=False)
    return f


def mesh(f, root, name):
    return f.require_group(f'{root}/{name}')


def geometry(path):
    with new_file(path) as f:
        g = mesh(f, GEOMETRY, 'Mesh A')
        coords(g, len(SPECIAL))
        elevation(g, 'FacePoints Elevation', SPECIAL)
        g = mesh(f, GEOMETRY, 'Mesh B')
        coords(g, 4)
        g = mesh(f, GEOMETRY, 'Mesh C')
        coords(g, len(PLAIN))
        elevation(g, 'FacePoints Elevation', PLAIN, chunked=False)


def cache(path):
    with new_file(path) as f:
        g = mesh(f, CACHE, 'Mesh A')
        elevation(g.require_group('Processed Data/Profile (Horizontal)'), 'FacePoint Elevation', SPECIAL)


def refusals(path):
    with new_file(path) as f:
        def m(name, n=3, width=2, with_coords=True):
            g = mesh(f, GEOMETRY, name)
            if with_coords:
                coords(g, n, width)
            return g
        elevation(m('Length Mismatch', n=4), 'FacePoints Elevation', PLAIN)
        elevation(m('Wide Float'), 'FacePoints Elevation', PLAIN, dtype='<f8')
        elevation(m('Big Endian'), 'FacePoints Elevation', PLAIN, dtype='>f4')
        elevation(m('Integer'), 'FacePoints Elevation', [1, 2, 3], dtype='<i4')
        elevation(m('Rank Two'), 'FacePoints Elevation', PLAIN, shape=(3, 1))
        g = m('Scalar', n=1)
        ds = g.create_dataset('FacePoints Elevation', data=np.float32(5.0), track_times=False)
        can_plot(ds)
        elevation(m('No Coordinates', with_coords=False), 'FacePoints Elevation', PLAIN)
        elevation(m('Wide Coordinates', width=3), 'FacePoints Elevation', PLAIN)
        elevation(m('Valid'), 'FacePoints Elevation', PLAIN)


geometry(a.output / 'geometry.h5')
cache(a.output / 'cache.h5')
refusals(a.output / 'refusals.h5')

EXPECT = {
    'geometry.h5': {
        f'{GEOMETRY}/Mesh A/FacePoints Elevation': ('supported', None),
        f'{GEOMETRY}/Mesh C/FacePoints Elevation': ('supported', None),
    },
    'cache.h5': {
        # A cache stores no FacePoints Coordinate: only a typed read with the plan's count qualifies.
        f'{CACHE}/Mesh A/{CACHE_DATASET}': ('unsupported', 'identity_layout'),
    },
    'refusals.h5': {
        f'{GEOMETRY}/Length Mismatch/FacePoints Elevation': ('corrupt', None),
        f'{GEOMETRY}/Wide Float/FacePoints Elevation': ('unsupported', 'encoding'),
        f'{GEOMETRY}/Big Endian/FacePoints Elevation': ('unsupported', 'encoding'),
        f'{GEOMETRY}/Integer/FacePoints Elevation': ('unsupported', 'encoding'),
        f'{GEOMETRY}/Rank Two/FacePoints Elevation': ('unsupported', 'schema'),
        f'{GEOMETRY}/Scalar/FacePoints Elevation': ('unsupported', 'schema'),
        f'{GEOMETRY}/No Coordinates/FacePoints Elevation': ('corrupt', None),
        f'{GEOMETRY}/Wide Coordinates/FacePoints Elevation': ('corrupt', None),
        f'{GEOMETRY}/Valid/FacePoints Elevation': ('supported', None),
    },
}
manifest = []
for name, datasets in EXPECT.items():
    path = a.output / name
    entry = {'name': name, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'datasets': []}
    with h5py.File(path, 'r') as f:
        for ds_path, (status, code) in sorted(datasets.items()):
            row = {'path': ds_path, 'status': status, 'code': code}
            if status == 'supported':
                values = f[ds_path][()]
                row.update(corners=int(f[ds_path.rsplit('/', 1)[0] + '/FacePoints Coordinate'].shape[0]),
                           h5py_dtype=values.dtype.str, h5py_hex=values.tobytes().hex())
            entry['datasets'].append(row)
    manifest.append(entry)
(a.output / 'manifest.json').write_text(json.dumps(manifest, indent=1) + '\n')
print(json.dumps(manifest, indent=1))
