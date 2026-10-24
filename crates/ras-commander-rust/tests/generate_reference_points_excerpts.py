"""Real point-geometry/series excerpts and explicit metadata/shape mutations."""
from pathlib import Path
import argparse,hashlib,json
import h5py,numpy as np
p=argparse.ArgumentParser();p.add_argument('--fixture',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
base='Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series';point=base+'/Reference Points'
with h5py.File(a.fixture) as src:
 paths=['Geometry/Reference Points/Attributes','Geometry/Reference Points/Points','Geometry/2D Flow Areas/Attributes','Geometry/2D Flow Areas/BaldEagleCr/Cells Center Coordinate',base+'/Time',base+'/Time Date Stamp',base+'/Time Date Stamp (ms)']+[point+'/'+v for v in ['Name','Water Surface','Velocity','Direction']]
 for mode in ['valid','reordered','bad_cells','bad_units','bad_name','bad_points','unknown_variable','bad_time','bad_areas','duplicate_geometry']:
  with h5py.File(a.output/(mode+'.h5'),'w',libver='earliest') as dst:
   for path in paths:
    if path not in src:continue
    x=src[path];data=x[()]
    if mode=='bad_points' and path=='Geometry/Reference Points/Points':data=data[:2]
    if mode=='duplicate_geometry' and path=='Geometry/Reference Points/Attributes':data['Name'][2]=data['Name'][0]
    if mode=='reordered' and path.startswith(point+'/'):data=data[[2,0,1]] if path.endswith('/Name') else data[:,[2,0,1]]
    if mode=='bad_name' and path==point+'/Name':data[0]=b'not_a_point'
    if mode=='bad_time' and path==base+'/Time':data=data[:2]
    y=dst.create_dataset(path,data=data,dtype=x.dtype,compression='gzip')
    for k,v in x.attrs.items():
     if mode=='reordered' and path.startswith(point+'/') and k in ['Cells','2D Areas']:v=v[[2,0,1]]
     if mode=='bad_cells' and path==point+'/Direction' and k=='Cells':v=v.copy();v[0]=0
     if mode=='bad_areas' and path==point+'/Direction' and k=='2D Areas':v=v.copy();v[0]=5
     if mode=='bad_units' and path==point+'/Direction' and k=='Units':v=np.bytes_('rad')
     y.attrs[k]=v
   if mode=='unknown_variable':dst.copy(point+'/Water Surface',point+'/Flow')
(a.output/'provenance.json').write_text(json.dumps(dict(source_name=a.fixture.name,source_sha256=hashlib.sha256(a.fixture.read_bytes()).hexdigest(),mutations='full stored Base arrays; explicit column permutation, wrong Cells/2D Areas/Units/Name, missing point row, Time mismatch, unknown Flow, duplicate geometry identity',files={f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in a.output.glob('*.h5')}),indent=2))
