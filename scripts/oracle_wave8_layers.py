"""Read-only h5py geometry-layer oracle. Writes only OUT; never changes fixtures.

Usage:
    python oracle_wave8_layers.py OUT.json FIXTURE.hdf [FIXTURE.hdf ...]

If no fixture paths are given on the command line, they are read from the
RAS_ORACLE_FIXTURES environment variable (paths separated by os.pathsep,
i.e. ';' on Windows and ':' elsewhere). Typical inputs are HEC-RAS plan HDF
files from the public HEC-RAS example projects (for example
BaldEagleDamBrk.p07.hdf, BaldEagleDamBrk.p18.hdf, DavisStormSystem.p02.hdf),
public FEMA eBFE / ras2fim models, and the Homewood model (Homewood.p02.hdf).

Numeric hashes cover exact little-endian stored values; samples are hex bytes.
"""
import hashlib
import json
import os
import sys
import math
from pathlib import Path
import h5py
import numpy as np

ENV_VAR = 'RAS_ORACLE_FIXTURES'


def fixture_paths(argv):
    paths = list(argv)
    if not paths:
        paths = [p for p in os.environ.get(ENV_VAR, '').split(os.pathsep) if p]
    if not paths:
        sys.exit(f'usage: {Path(__file__).name} OUT.json FIXTURE.hdf [...] '
                 f'(or set {ENV_VAR})')
    return paths


def main():
    if len(sys.argv) < 2:
        sys.exit(f'usage: {Path(__file__).name} OUT.json FIXTURE.hdf [...]')
    out = {'generator': Path(__file__).name, 'h5py': h5py.__version__, 'files': []}
    for name in fixture_paths(sys.argv[2:]):
        p = Path(name)
        rec = {'path': name, 'datasets': []}
        out['files'].append(rec)
        if not p.is_file():
            rec['error'] = 'file unavailable'
            continue
        rec['sha256'] = hashlib.file_digest(p.open('rb'), 'sha256').hexdigest()
        try:
            with h5py.File(p, 'r') as f:
                def visit(path, obj):
                    if not isinstance(obj, h5py.Dataset):
                        return
                    r = {'path': 'Geometry/' + path, 'shape': list(obj.shape), 'dtype': str(obj.dtype)}
                    rec['datasets'].append(r)
                    if obj.dtype.kind in 'iuf':
                        a = np.asarray(obj[...], dtype=obj.dtype.newbyteorder('<'))
                        r['sha256'] = hashlib.sha256(a.tobytes()).hexdigest()
                        if path.endswith(('Polyline Info', 'Polygon Info')):
                            prefix = path[:-4]
                            parts = f['Geometry/' + prefix + 'Parts'][...]
                            r['features'] = [{'points': [int(i[0]), int(i[0]+i[1])],
                                              'parts': [[int(i[0]+p[0]), int(i[0]+p[0]+p[1])]
                                                        for p in parts[int(i[2]):int(i[2]+i[3])]]} for i in a]
                        if a.ndim and a.shape[0]:
                            for label, sl in [('head', slice(0, min(3, a.shape[0]))),
                                              ('tail', slice(max(0, a.shape[0]-2), a.shape[0]))]:
                                r[label] = {'start': sl.start, 'end': sl.stop, 'hex': a[sl].tobytes().hex()}
                    elif obj.dtype.names:
                        r['raw_sha256'] = hashlib.sha256(obj[...].tobytes()).hexdigest()
                        r['fields'] = list(obj.dtype.names)
                        r['rows'] = [{k: (v[k].decode('latin1').rstrip('\x00 ') if isinstance(v[k], bytes)
                                         else np.asarray(v[k]).tolist()) for k in obj.dtype.names} for v in obj[...]]
                if 'Geometry' in f:
                    f['Geometry'].visititems(visit)
        except Exception as e:
            rec['error'] = str(e)
    def finite(v):
        if isinstance(v, float) and not math.isfinite(v):
            return str(v)
        if isinstance(v, dict):
            return {k: finite(x) for k, x in v.items()}
        if isinstance(v, list):
            return [finite(x) for x in v]
        return v
    Path(sys.argv[1]).write_text(json.dumps(finite(out), indent=2, allow_nan=False))

if __name__ == '__main__':
    main()
