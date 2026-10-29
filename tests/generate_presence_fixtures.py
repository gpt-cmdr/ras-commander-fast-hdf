"""Synthetic layer-family presence cases (tests/family_presence.rs).

Each file stores families in a known state: complete, canonically absent, missing a dataset the
reader needs, or refused before any read. Values are minimal; only the layout matters.

    python tests/generate_presence_fixtures.py --output tests/fixtures/presence
"""
from pathlib import Path
import argparse, hashlib, json
import h5py, numpy as np

LINES = {  # 1D packed line families: group, prefix
    'xs': ('Geometry/Cross Sections', 'Polyline'),
    'centerlines': ('Geometry/River Centerlines', 'Polyline'),
    'banks': ('Geometry/River Bank Lines', 'Polyline'),
    'edges': ('Geometry/River Edge Lines', 'Polyline'),
    'flow_paths': ('Geometry/River Flow Paths', 'Flow Path Lines'),
    'structures': ('Geometry/Structures', 'Centerline'),
    'storage': ('Geometry/Storage Areas', 'Polygon'),
    'conduits': ('Geometry/Pipe Conduits', 'Polyline'),
}
GEOMETRY = {  # packed 2D geometry families
    'bc_lines': ('Geometry/Boundary Condition Lines', 'Polyline'),
    'breaklines': ('Geometry/2D Flow Area Break Lines', 'Polyline'),
    'refinement': ('Geometry/2D Flow Area Refinement Regions', 'Polygon'),
    'reference_lines': ('Geometry/Reference Lines', 'Polyline'),
    'calibration': ("Geometry/Land Cover (Manning's n)", 'Polygon'),
}
POINTS = ['Geometry/Junctions', 'Geometry/Pump Stations', 'Geometry/Pipe Nodes']

NAMED = np.dtype([('Name', 'S16')])
UNNAMED = np.dtype([('Label', 'S16')])


def packed(f, group, prefix, drop=(), attributes=NAMED, extra=()):
    g = f.require_group(group)
    parts = {
        'Attributes': lambda: g.create_dataset('Attributes', data=np.array([(b'one',)], dtype=attributes)),
        'Info': lambda: g.create_dataset(f'{prefix} Info', data=np.array([[0, 2, 0, 1]], dtype='<i4')),
        'Parts': lambda: g.create_dataset(f'{prefix} Parts', data=np.array([[0, 2]], dtype='<i4')),
        'Points': lambda: g.create_dataset(f'{prefix} Points', data=np.array([[0.0, 0.0], [1.0, 1.0]], dtype='<f8')),
    }
    for name, make in parts.items():
        if name not in drop:
            make()
    for name in extra:
        g.create_dataset(name, data=np.array([b'x'], dtype='S4'))


def point_family(f, group, drop=()):
    g = f.require_group(group)
    if 'Attributes' not in drop:
        g.create_dataset('Attributes', data=np.array([(b'one',)], dtype=NAMED))
    if 'Points' not in drop:
        g.create_dataset('Points', data=np.array([[0.0, 0.0]], dtype='<f8'))


def reference_points(f, drop=()):
    g = f.require_group('Geometry/Reference Points')
    dt = np.dtype([('Name', 'S16'), ('SA/2D', 'S16'), ('Cell Index', '<i4')])
    if 'Attributes' not in drop:
        g.create_dataset('Attributes', data=np.array([(b'rp', b'Area', 0)], dtype=dt))
    if 'Points' not in drop:
        g.create_dataset('Points', data=np.array([[0.0, 0.0]], dtype='<f8'))


def meshes(f, names, perimeter=lambda name: True):
    g = f.require_group('Geometry/2D Flow Areas')
    for name in names:
        m = g.create_group(name)
        if perimeter(name):
            m.create_dataset('Perimeter', data=np.array([[0, 0], [1, 0], [1, 1]], dtype='<f8'))


def stored(f):
    """Every family complete."""
    for group, prefix in LINES.values():
        packed(f, group, prefix)
    for group, prefix in GEOMETRY.values():
        packed(f, group, prefix)
    for group in POINTS:
        point_family(f, group)
    reference_points(f)
    meshes(f, ['Area A', 'Area B'])


def empty(f):
    """No family stored; an empty Geometry group."""
    f.create_group('Geometry')


def incomplete(f):
    """Each family absent, incomplete or refused in one way."""
    L, G = LINES, GEOMETRY
    packed(f, *L['xs'], drop=('Parts',))                        # NotFound: absent
    packed(f, *L['centerlines'], drop=('Points',))              # NotFound: absent
    packed(f, *L['banks'], drop=('Info',))                      # canonical absence
    packed(f, *L['edges'], drop=('Info', 'Parts', 'Points', 'Attributes'))  # empty group
    packed(f, *L['flow_paths'], drop=('Attributes',))           # optional Attributes: stored
    packed(f, *L['structures'], drop=('Attributes', 'Parts'))   # required Attributes and Parts missing: absent
    packed(f, *L['storage'], drop=('Attributes',))              # required Attributes missing: refused, stored
    packed(f, *G['bc_lines'], drop=('Attributes',))             # NotFound: absent
    packed(f, *G['breaklines'], attributes=UNNAMED, drop=('Parts',))  # no Name column and no Parts: absent
    packed(f, *G['refinement'], attributes=UNNAMED)             # no Name column: refused, stored
    packed(f, *G['reference_lines'], drop=('Info',))            # NotFound: absent
    f.require_group(G['calibration'][0]).create_dataset('Calibration Table', data=np.zeros(1))  # no polygons: absent
    reference_points(f, drop=('Points',))                       # NotFound: absent
    point_family(f, 'Geometry/Junctions', drop=('Points',))     # one of the pair: corrupt, stored
    f.require_group('Geometry/Pump Stations')                   # neither of the pair: absent
    meshes(f, ['Area A', 'Area B'], perimeter=lambda n: n == 'Area A')  # one Perimeter missing


def legacy_xs(f):
    """A separated (legacy) cross-section layout that also lacks Parts: absent, not refused."""
    packed(f, *LINES['xs'], drop=('Parts',), extra=('River Stations',))
    f.require_group('Geometry/2D Flow Areas')                   # no mesh groups: refused, stored


CASES = dict(presence_stored=stored, presence_empty=empty, presence_incomplete=incomplete,
             presence_legacy_xs=legacy_xs)

if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    for name, build in CASES.items():
        with h5py.File(a.output / f'{name}.h5', 'w', track_order=True) as f:
            build(f)
    (a.output / 'provenance.json').write_bytes((json.dumps(dict(
        source='synthetic h5py %s (tests/generate_presence_fixtures.py)' % h5py.__version__,
        files={q.name: hashlib.sha256(q.read_bytes()).hexdigest() for q in sorted(a.output.glob('*.h5'))}),
        indent=2) + '\n').encode())
