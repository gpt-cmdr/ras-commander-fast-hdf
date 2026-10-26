"""h5py oracle for the real-file corner-elevation test (`tests/corner_elevations.rs`).

Lists every 2D mesh corner-elevation array in the given HEC-RAS files with h5py and records,
per array, its layout, corner count and the SHA-256 of the exact bytes h5py returns for
`dataset[()]` (little-endian f4). Geometry-layout counts come from the same file's
`FacePoints Coordinate`; a mapping cache (`PostProcessing.hdf`) is paired with the plan HDF
whose corner counts it must match (`--cache CACHE=PLAN`). Arrays the engine must refuse are
recorded with their `native_status`.

Usage:
  python corner_elevation_oracle.py --output oracle.json [--hdf FILE ...] [--cache CACHE=PLAN ...]
  RAS_CORNER_ELEVATION_ORACLE=oracle.json cargo test --test corner_elevations -- --ignored
"""
import argparse, hashlib, json
import h5py

GEOMETRY = 'Geometry/2D Flow Areas'
CACHE = 'Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas'
CACHE_DATASET = 'Processed Data/Profile (Horizontal)/FacePoint Elevation'
# Keep in step with MAPPING_CACHE_QUALIFIED in src/hdf/corner_elevations.rs.
CACHE_NATIVE_STATUS = 'supported'


def corner_counts(path):
    with h5py.File(path, 'r') as f:
        root = f.get(GEOMETRY)
        return {} if root is None else {
            m: int(g['FacePoints Coordinate'].shape[0]) for m, g in root.items()
            if isinstance(g, h5py.Group) and 'FacePoints Coordinate' in g}


def arrays(path, root, relative, layout, counts):
    out = []
    with h5py.File(path, 'r') as f:
        group = f.get(root)
        for mesh in sorted(group) if group is not None else []:
            ds = group[mesh].get(relative) if isinstance(group[mesh], h5py.Group) else None
            if not isinstance(ds, h5py.Dataset):
                continue
            values = ds[()]
            if values.dtype != '<f4' or values.ndim != 1:
                raise SystemExit(f'{path}: {mesh} {relative} is {values.dtype} {values.shape}; not an observed layout')
            row = {'mesh': mesh, 'layout': layout, 'corners': counts.get(mesh),
                   'h5py_sha256': hashlib.sha256(values.tobytes()).hexdigest()}
            if row['corners'] != values.shape[0]:
                raise SystemExit(f'{path}: {mesh} has {values.shape[0]} values for {row["corners"]} corners')
            if layout == 'mapping_cache':
                row['native_status'] = CACHE_NATIVE_STATUS
            out.append(row)
    return out


p = argparse.ArgumentParser()
p.add_argument('--output', required=True)
p.add_argument('--hdf', nargs='*', default=[])
p.add_argument('--cache', nargs='*', default=[])
a = p.parse_args()
files = []
for path in a.hdf:
    found = arrays(path, GEOMETRY, 'FacePoints Elevation', 'geometry', corner_counts(path))
    if found:
        files.append({'path': path, 'arrays': found})
for pair in a.cache:
    cache, plan = pair.split('=', 1)
    found = arrays(cache, CACHE, CACHE_DATASET, 'mapping_cache', corner_counts(plan))
    if found:
        files.append({'path': cache, 'plan': plan, 'arrays': found})
json.dump({'files': files}, open(a.output, 'w'), indent=1)
print(f'{sum(len(f["arrays"]) for f in files)} arrays in {len(files)} files')
