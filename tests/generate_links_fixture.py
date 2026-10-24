"""Synthetic link cases (review P3-6): soft links to datasets in old-style (symbol-table) and
new-style groups, including the root group, plus dangling, group-target and external links."""
from pathlib import Path
import argparse,hashlib,json
import h5py,numpy as np
p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
for lib in ['earliest','latest']:
 with h5py.File(a.output/f'links_{lib}.h5','w',libver=lib) as f:
  d=f.create_dataset('g/d',data=np.arange(3,dtype='<f4'));d.attrs['u']=np.bytes_(b'ft')
  f.create_dataset('g/s',data=np.array([b'ab'],dtype='S4'))
  f['g/soft']=h5py.SoftLink('/g/d');f['soft_root']=h5py.SoftLink('/g/d');f['g/soft_str']=h5py.SoftLink('/g/s')
  f['g/dangling']=h5py.SoftLink('/g/nope');f['g/soft_group']=h5py.SoftLink('/g')
  f['g/ext']=h5py.ExternalLink('absent_other.h5','/x')
(a.output/'provenance.json').write_bytes((json.dumps(dict(source='synthetic h5py %s'%h5py.__version__,
 files={q.name:hashlib.sha256(q.read_bytes()).hexdigest() for q in sorted(a.output.glob('*.h5'))}),indent=2)+'\n').encode())
