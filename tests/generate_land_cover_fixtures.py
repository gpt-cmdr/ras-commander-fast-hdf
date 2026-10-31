"""Synthetic land-cover and gridded-precipitation files (tests/land_cover_precipitation.rs).

Values are small and chosen by hand; the expected results are written out in the test.

    python tests/generate_land_cover_fixtures.py --output tests/fixtures/land_cover
"""
from pathlib import Path
import argparse, hashlib, json
import h5py, numpy as np

LC = "Geometry/Land Cover (Manning's n)"
SQUARE = [[0.0, 0.0], [0.0, 8.0], [8.0, 8.0], [8.0, 0.0], [0.0, 0.0]]
REGION = [[0.0, 4.0], [0.0, 8.0], [4.0, 8.0], [4.0, 4.0], [0.0, 4.0]]


def polygon(f, group, rings):
    g = f.require_group(group)
    pts = np.array([p for r in rings for p in r], dtype='<f8')
    info, at = [], 0
    for r in rings:
        info.append([at, len(r), 0, 1])
        at += len(r)
    g.create_dataset('Polygon Info', data=np.array(info, dtype='<i4'))
    g.create_dataset('Polygon Parts', data=np.array([[i[0], i[1]] for i in info], dtype='<i4'))
    g.create_dataset('Polygon Points', data=pts)


def geometry(path, land_cover=b'.\\lc.hdf', cross_sections=False, multipart=False):
    with h5py.File(path, 'w') as f:
        g = f.create_group('Geometry')
        g.attrs['Land Cover Filename'] = np.bytes_(land_cover)
        d2 = f.create_group('Geometry/2D Flow Areas')
        d2.create_dataset('Attributes', data=np.array(
            [(b'Area', 0.5, 16)], dtype=[('Name', 'S8'), ('Mann', '<f4'), ('Cell Count', '<i4')]))
        polygon(f, 'Geometry/2D Flow Areas', [SQUARE])
        if multipart:
            d2['Polygon Info'][0, 3] = 2
        f.create_group(LC).create_dataset('Attributes', data=np.array(
            [(b'West',)], dtype=[('Name', 'S8')]))
        polygon(f, LC, [REGION])
        f[LC].create_dataset('Calibration Table', data=np.array(
            [(b'Grass', 0.04, 0.2), (b'Forest', -9999.0, -9999.0)],
            dtype=[('Land Cover Name', 'S8'), ("Base Manning's n Value", '<f4'), ('West', '<f4')]))
        if cross_sections:
            f.create_group('Geometry/Cross Sections').create_dataset(
                'Attributes', data=np.array([(b'XS',)], dtype=[('River', 'S8')]))


def land_cover(path):
    with h5py.File(path, 'w') as f:
        f.create_dataset('Raster Map', data=np.array(
            [(0, b'NoData'), (1, b'Grass'), (7, b'Forest')], dtype=[('ID', 'u1'), ('Name', 'S8')]))
        f.create_dataset('Variables', data=np.array(
            [(b'NoData', -9999.0), (b'Grass', 0.03), (b'Forest', 0.1)],
            dtype=[('Name', 'S8'), ('ManningsN', '<f4')]))


def land_cover_v1_bitfield(path):
    """The version-1 layout HEC-RAS writes, whose IDs are a one-byte bitfield."""
    with h5py.File(path, 'w') as f:
        space = h5py.h5s.create_simple((3,))
        ds = h5py.h5d.create(f.id, b'IDs', h5py.h5t.STD_B8LE, space)
        ds.write(h5py.h5s.ALL, h5py.h5s.ALL, np.array([0, 1, 7], dtype='u1'),
                 mtype=h5py.h5t.NATIVE_UINT8)
        f.create_dataset('Names', data=np.array([b'NoData', b'Grass', b'Forest'], dtype='S8'))
        f.create_dataset('ManningsN', data=np.array([-9999.0, 0.03, 0.1], dtype='<f4'))


def land_cover_virtual(path):
    land_cover(path)
    with h5py.File(path, 'a') as f:
        f.attrs['Virtual Dependencies'] = np.bytes_(b'a.hdf')


def precipitation(path, values, data_type=b'per-cum', nodata=None, units=b'in', timestamps=True,
                  chunks=None):
    with h5py.File(path, 'w') as f:
        g = f.create_group('Event Conditions/Meteorology/Precipitation')
        g.attrs['Raster Rows'] = 2
        g.attrs['Raster Cols'] = 3
        g.attrs['Raster Cellsize'] = np.float32(0.1)
        g.attrs['Raster Left'] = 100.0
        g.attrs['Raster Top'] = 200.0
        g.attrs['Projection'] = np.bytes_(b'PROJCS["x"]')
        g.attrs['Units'] = np.bytes_(units)
        g.attrs['Data Type'] = np.bytes_(data_type)
        if nodata is not None:
            g.attrs['NoData'] = np.float32(nodata)
        v = np.array(values, dtype='<f4')
        g.create_dataset('Values', data=v, chunks=chunks)
        if timestamps:
            g.create_dataset('Timestamp', data=np.array(
                [f'0{i + 1}JAN2000 00:00:00'.encode() for i in range(v.shape[0])], dtype='S22'))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--output', type=Path, required=True)
    out = ap.parse_args().output
    out.mkdir(parents=True, exist_ok=True)
    geometry(out / 'geom.hdf')
    geometry(out / 'geom_v1.hdf', land_cover=b'.\\lc_v1.hdf')
    geometry(out / 'geom_virtual.hdf', land_cover=b'.\\lc_virtual.hdf')
    geometry(out / 'geom_xs.hdf', cross_sections=True)
    geometry(out / 'geom_multipart.hdf', multipart=True)
    land_cover(out / 'lc.hdf')
    land_cover_v1_bitfield(out / 'lc_v1.hdf')
    land_cover_virtual(out / 'lc_virtual.hdf')
    per = [[1, 2, -9999, 4, 5, -9999], [1, -9999, -9999, 4, 5, 6], [1, 2, -9999, 4, 5, 6]]
    precipitation(out / 'precip_per.hdf', per, nodata=-9999, chunks=(2, 6))
    precipitation(out / 'precip_cumulative.hdf', [[1, 1, 1, 1, 1, 1], [3, 2, 1, 4, 5, 6]],
                  data_type=b'cumulative', units=b'mm', timestamps=False)
    precipitation(out / 'precip_full.hdf', [[1, 1, 1, 1, 1, 1], [2, 2, 2, 2, 2, 2]])
    files = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
             for p in sorted(out.iterdir()) if p.suffix == '.hdf'}
    (out / 'manifest.json').write_text(json.dumps(
        {'source': f'synthetic h5py {h5py.__version__} (tests/generate_land_cover_fixtures.py)',
         'files': files}, indent=2) + '\n')


if __name__ == '__main__':
    main()
