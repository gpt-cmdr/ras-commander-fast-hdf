"""Supplemental small malformed/mismatch cases; real geometry qualification uses the oracle.
Usage: python generate_wave8_cases.py OUT_DIR. Creates files only under OUT_DIR.
"""
import hashlib
import json
import sys
from pathlib import Path
import h5py
import numpy as np

out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
for case in ['base', 'centers', 'faces', 'normals', 'shape', 'kind', 'bad_offsets', 'bad_shape', 'wrong_object', 'blank_name']:
    path = out / (case + '.hdf')
    with h5py.File(path, 'w') as f:
        root = f.create_group('Geometry/2D Flow Areas')
        root['Attributes'] = np.array([(b'A', 2)], dtype=[('Name', 'S8'), ('Cell Count', '<i4')])
        a = root.create_group('A')
        centers = np.array([[1., 2.], [3., 4.], [5., 6.]], dtype='<f8')
        faces = np.array([[0, 1], [1, 2]], dtype='<i4')
        normals = np.array([[1., 0., 10.], [0., 1., 20.]], dtype='<f4')
        if case == 'centers': centers[1, 1] = 4.00001
        if case == 'faces': faces[0, 1] = 2
        if case == 'normals': normals[1, 2] = 21
        if case == 'shape': faces = faces[:1]
        if case == 'kind': faces = faces.astype('<i8')
        a['Cells Center Coordinate'] = centers
        a['Faces Cell Indexes'] = faces
        a['Faces NormalUnitVector and Length'] = normals
        if case == 'wrong_object':
            f['Geometry/Boundary Condition Lines'] = np.array([1], dtype='<i4')
        else:
            g = f.create_group('Geometry/Boundary Condition Lines')
            g['Attributes'] = np.array([(b' ' if case == 'blank_name' else b'BC',)], dtype=[('Name', 'S8')])
            g['Polyline Info'] = np.array([[0, 2, 0, 1]], dtype='<i4')
            g['Polyline Parts'] = np.array([[0, 3 if case == 'bad_offsets' else 2]], dtype='<i4')
            g['Polyline Points'] = np.array([[0., 1.], [1., 2.]], dtype='<f8') if case != 'bad_shape' else np.zeros((2, 3))
hashes = {p.name: hashlib.file_digest(p.open('rb'), 'sha256').hexdigest() for p in out.glob('*.hdf')}
(out/'manifest.json').write_text(json.dumps({'generator': __file__, 'sha256': hashes}, indent=2))
