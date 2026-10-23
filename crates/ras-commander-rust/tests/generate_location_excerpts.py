"""Real Davis/Austin excerpts; explicit Location and identity mutations supplement full real oracles."""
from pathlib import Path
import argparse,hashlib,json
import h5py,numpy as np
p=argparse.ArgumentParser();p.add_argument('--fixture-dir',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
base='Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series'
def copy(src,dst,path,target=None,data=None):
 x=src[path];y=dst.create_dataset(target or path,data=x[()] if data is None else data,dtype=x.dtype,compression='gzip')
 for k,v in x.attrs.items():y.attrs[k]=v
 return y
with h5py.File(a.fixture_dir/'DavisStormSystem.p02.hdf') as src:
 for mode in ['valid','missing','unknown','mismatch','float_ids','missing_table','short_ids']:
  with h5py.File(a.output/('pipe_'+mode+'.h5'),'w',libver='earliest') as dst:
   for name in ['Time','Time Date Stamp','Time Date Stamp (ms)']:copy(src,dst,base+'/'+name,data=src[base+'/'+name][:2])
   geo='Geometry/Pipe Networks/Davis/'
   for name in ['Node Indices','Conduit Indices','Cell Polygons Info','Face Polylines Info']:
    if name=='Node Indices' and mode=='missing_table':continue
    data=src[geo+name][()];data=data[:133] if name in ['Cell Polygons Info','Face Polylines Info'] else data
    if name=='Node Indices' and mode=='short_ids':data=data[:132]
    target='Geometry/Pipe Networks/N/'+name
    if name=='Node Indices' and mode=='float_ids':dst.create_dataset(target,data=data.astype('<f4'))
    else:copy(src,dst,geo+name,target,data)
   x=copy(src,dst,base+'/Pipe Networks/Davis/Nodes/Depth',base+'/Pipe Networks/N/Face Velocity',src[base+'/Pipe Networks/Davis/Nodes/Depth'][:2])
   if mode=='missing':del x.attrs['Location']
   if mode=='unknown':x.attrs['Location']=np.bytes_('Unknown domain')
   if mode=='mismatch':x.attrs['Location']=np.bytes_('Pipe Links')
   copy(src,dst,base+'/Pipe Networks/Davis/Pipes/Pipe Flow US',base+'/Pipe Networks/N/Unrelated Quantity',src[base+'/Pipe Networks/Davis/Pipes/Pipe Flow US'][:2])
with h5py.File(a.fixture_dir/'AustinOyster.p01.hdf') as src:
 area='Perimeter 1';geo='Geometry/2D Flow Areas/'+area+'/'
 for mode in ['valid','missing','unknown','mismatch','singular']:
  with h5py.File(a.output/('mesh_'+mode+'.h5'),'w',libver='earliest') as dst:
   for name in ['Time','Time Date Stamp','Time Date Stamp (ms)']:copy(src,dst,base+'/'+name,data=src[base+'/'+name][:2])
   for name in ['Cells Center Coordinate','Faces FacePoint Indexes','FacePoints Coordinate']:copy(src,dst,geo+name,data=src[geo+name][:4 if mode=='mismatch' and name=='Faces FacePoint Indexes' else 3])
   root=base+'/2D Flow Areas/'+area+'/'
   x=copy(src,dst,root+'Water Surface',root+'Face Velocity',src[root+'Water Surface'][:2,:3])
   if mode=='missing':del x.attrs['Location']
   if mode=='unknown':x.attrs['Location']=np.bytes_('Nodes')
   if mode=='mismatch':x.attrs['Location']=np.bytes_('Faces')
   if mode=='singular':x.attrs['Location']=np.bytes_('Cell')
   copy(src,dst,root+'Face Velocity',root+'Cell Level',src[root+'Face Velocity'][:2,:3])
   summary=base.replace('Unsteady Time Series','Summary Output')+'/2D Flow Areas/'+area+'/'
   copy(src,dst,summary+'Maximum Water Surface',data=src[summary+'Maximum Water Surface'][:,:3])
(a.output/'provenance.json').write_text(json.dumps(dict(sources={n:hashlib.sha256((a.fixture_dir/n).read_bytes()).hexdigest() for n in ['DavisStormSystem.p02.hdf','AustinOyster.p01.hdf']},mutations='first two time rows, renamed network/variables, coincident cell/face/node counts, missing/unknown/contradictory Location, float/missing/short stored IDs, observed singular Cell alias; original literal values retained',files={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in a.output.glob('*.h5')}),indent=2))
