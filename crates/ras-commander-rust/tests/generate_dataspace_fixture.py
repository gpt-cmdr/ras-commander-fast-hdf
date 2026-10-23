"""Synthetic h5py dataspace cases: NULL (h5py.Empty), scalar, and zero-length
simple arrays, as attributes on a group and a dataset and as datasets. No real data involved."""
from pathlib import Path
import argparse,hashlib,json
import h5py,numpy as np
p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
compound=np.dtype([('Name','S4'),('Value','<f4')])
def null_bitfield(owner,name):
 t=h5py.h5t.STD_B8LE.copy();s=h5py.h5s.create(h5py.h5s.NULL)
 h5py.h5a.create(owner.id,name.encode(),t,s)
def attrs(o):
 o.attrs['null_f4']=h5py.Empty('<f4');o.attrs['null_i4']=h5py.Empty('<i4');o.attrs['null_str']=h5py.Empty('S4')
 o.attrs['null_compound']=h5py.Empty(compound);null_bitfield(o,'null_bitfield')
 o.attrs['scalar_f4']=np.float32(1.5);o.attrs['scalar_str']=np.bytes_(b'ab')
 o.attrs['empty_f4']=np.array([],dtype='<f4');o.attrs['empty_str']=np.array([],dtype='S4')
 o.attrs['empty_2d_i4']=np.zeros((0,3),dtype='<i4');o.attrs['one_f4']=np.array([2.5],dtype='<f4')
with h5py.File(a.output/'dataspace.h5','w',libver='earliest') as f:
 attrs(f.create_group('g'))
 d=f.create_dataset('d',data=np.arange(3,dtype='<f4'));attrs(d)
 s=f.create_group('datasets')
 for k,v in [('null_f4',h5py.Empty('<f4')),('null_str',h5py.Empty('S4')),('null_compound',h5py.Empty(compound)),
             ('scalar_f4',np.float32(1.5)),('scalar_str',np.bytes_(b'ab')),('scalar_compound',np.array((b'ab',1.5),dtype=compound)),
             ('empty_f4',np.array([],dtype='<f4')),('empty_str',np.array([],dtype='S4')),('empty_compound',np.array([],dtype=compound)),
             ('empty_2d_f4',np.zeros((0,3),dtype='<f4'))]:
  s.create_dataset(k,data=v)
(a.output/'provenance.json').write_text(json.dumps(dict(source='synthetic h5py %s'%h5py.__version__,
 cases='NULL (h5py.Empty) / scalar / zero-length simple attributes on /g and /d, and the same as datasets under /datasets; NULL bitfield attribute via h5a.create',
 files={q.name:hashlib.sha256(q.read_bytes()).hexdigest() for q in sorted(a.output.glob('*.h5'))}),indent=2)+'\n')
