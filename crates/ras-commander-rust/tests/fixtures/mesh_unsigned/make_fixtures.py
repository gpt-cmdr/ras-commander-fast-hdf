"""Mesh attribute tables with an unsigned Cell Count; run: python -I make_fixtures.py OUTDIR"""
from pathlib import Path
import sys, json, hashlib
import h5py, numpy as np

out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
for case, dtype, count in (('unsigned', '<u4', 2), ('unsigned_huge', '<u8', 2**63 + 5)):
    with h5py.File(out / (case + '.h5'), 'w', libver='earliest') as f:
        g = f.create_group('Geometry/2D Flow Areas')
        g['Attributes'] = np.array([(b'A', count)], dtype=[('Name', 'S8'), ('Cell Count', dtype)])
        g['A/Cells Center Coordinate'] = np.zeros((3, 2), dtype='<f8')
(out / 'manifest.json').write_text(json.dumps(
    {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.glob('*.h5'))}, indent=2) + '\n')
