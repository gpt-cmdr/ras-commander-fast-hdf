"""Read-only real excerpts, with explicit review mutations; never compute HEC-RAS."""
from pathlib import Path
import h5py,hashlib,json,numpy as np
import argparse
parser=argparse.ArgumentParser();parser.add_argument('--fixture-dir',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
src=args.fixture_dir;out=args.output;out.mkdir(parents=True,exist_ok=True)
OUTPUT='Results/Unsteady/Output/Output Blocks';base=OUTPUT+'/Base Output/Unsteady Time Series'
with h5py.File(src/'BaldEagleDamBrk.p07.hdf') as f,h5py.File(out/'budget_messages.h5','w',libver='earliest') as d:
 p='Results/Summary/Compute Messages (rtf)';x=f[p]
 for n in [p,OUTPUT+'/Review Copy/Compute Messages (rtf)']:
  y=d.create_dataset(n,data=x[()],dtype=x.dtype,compression='gzip' if x.shape else None)
 d.create_dataset('Plan Data/Review Small',data=np.array([7],dtype='<i4'))
 d.create_dataset(OUTPUT+'/Review Copy/Small',data=np.array([7],dtype='<i4'))
for mode in ['valid','unknown','duplicate_time','bad_units','bad_stamp','decreasing']:
 with h5py.File(src/'DavisStormSystem.p02.hdf') as f,h5py.File(out/('pipe_'+mode+'.h5'),'w',libver='earliest') as d:
  for tail in ['Time','Time Date Stamp','Time Date Stamp (ms)','Pipe Networks/Davis/Nodes/Depth']:
   p=base+'/'+tail;x=f[p];data=x[:2]
   if tail=='Time' and mode=='duplicate_time':data[1]=data[0]
   if tail=='Time' and mode=='decreasing':data=data[::-1]
   if tail=='Time Date Stamp' and mode=='bad_stamp':data=data[:1]
   y=d.create_dataset(p,data=data,dtype=x.dtype)
   for k,v in x.attrs.items():y.attrs[k]=v
   if tail=='Time' and mode=='bad_units':y.attrs['Time']=np.bytes_('hours')
  for tail in ['Node Indices','Cell Polygons Info']:
   p='Geometry/Pipe Networks/Davis/'+tail;x=f[p];d.create_dataset(p,data=x[:133],dtype=x.dtype)
  if mode=='unknown':
   x=f[base+'/Pipe Networks/Davis/Nodes/Depth'];d.create_dataset(base+'/Pipe Networks/Davis/Maximum Node Depth',data=x[:2],dtype=x.dtype)
with h5py.File(src/'AustinOyster.p01.hdf') as f,h5py.File(out/'reference_literal.h5','w',libver='earliest') as dst:
 group=OUTPUT+'/DSS Hydrograph Output/Unsteady Time Series'
 for tail in ['Time','Reference Lines/Name','Reference Lines/Water Surface Max']:
  x=f[group+'/'+tail];y=dst.create_dataset(group+'/'+tail,data=x[()],dtype=x.dtype)
  for k,v in x.attrs.items():y.attrs[k]=v
(out/'provenance.json').write_text(json.dumps(dict(sources={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [src/'DavisStormSystem.p02.hdf',src/'BaldEagleDamBrk.p07.hdf',src/'AustinOyster.p01.hdf']},mutations='Budget message relocated into output block; pipe first two rows, Cell identity truncated to coincident 133 rows; renamed unknown node variable; separate duplicate/decreasing Time, invalid Time units, stamp length variants.',files={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in out.glob('*.h5')}),indent=2))
