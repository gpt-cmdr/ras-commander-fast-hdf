"""Generate synthetic reference/BC/calibration-region capability fixtures.
Run: python -I make_fixtures.py OUTDIR. Needs h5py/numpy. Writes *.h5 + manifest.json with SHA256
and hand-written expectations. No real fixture contains these groups (probe: 0 matches in 4 local
model files and 100 example HDFs), so the layouts below are deliberately minimal and not claimed real."""
import hashlib, json, sys
from pathlib import Path
import h5py, numpy as np

BASE = 'Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series'
FAMILIES = {'rl': 'Reference Lines', 'rp': 'Reference Points',
            'bc': 'Boundary Conditions', 'cr': 'Calibration Regions'}
U, X, C, A = 'Unsupported:IdentityLayout', None, 'Corrupt', 'Unavailable'


def build(path, states):
    with h5py.File(path, 'w') as f:
        g = f.require_group(BASE)
        g.create_dataset('Time', data=np.arange(4, dtype='<f8') / 24.0)
        for key, st in states.items():
            if st == 'group':
                g.create_group(FAMILIES[key]).create_dataset('x', data=np.zeros((4, 1), '<f4'))
            elif st == 'dataset':
                g.create_dataset(FAMILIES[key], data=np.zeros((4, 1), '<f4'))


CASES = {
    'all_absent': {},
    'all_present': {k: 'group' for k in FAMILIES},
    'all_wrong_type': {k: 'dataset' for k in FAMILIES},
    'mixed_isolation': {'rl': 'group', 'rp': 'dataset', 'bc': 'group'},
}
EXPECT = {
    'all_absent': dict(rl=A, rp=A, bc=A, cr=A),
    'all_present': dict(rl=U, rp='Unsupported:Schema', bc=U, cr=U),
    'all_wrong_type': dict(rl=C, rp=C, bc=C, cr=C),
    'mixed_isolation': dict(rl=U, rp=C, bc=U, cr=A),
}

out = Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
man = {'fixtures': {}}
for name, states in CASES.items():
    p = out / (name + '.h5'); build(p, states)
    man['fixtures'][name] = {'file': p.name, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest(),
                             'expect': EXPECT[name]}
(out / 'manifest.json').write_text(json.dumps(man, indent=1))
