"""Freeze immutable 6.6 and 7.0.1 reference-point outputs, without running native code or HEC-RAS."""
from pathlib import Path
import argparse,hashlib,json,sys
import h5py,numpy as np
from generate_wave3_oracle import freeze
p=argparse.ArgumentParser();p.add_argument('--fixture-dir',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();cases=[]
expected={'BaldEagleDamBrk.refpts.701.p03.hdf':'95ad4fccf2753676d02d77bb3a58cddd952c6bc77ecf1cf7398028ee6d92c185','BaldEagleDamBrk.g09.refpts.701.hdf':'5859d683d6f32e77275c28b2940567a8093ccc1c7723f3af138ab4545d518c7d'}
expected.update({'BaldEagleDamBrk.refpts.66.p03.hdf':'f08684a3c57c075ace9c2bf6fdda290bca428aebb5d437bfc04c206e55edb4ec','BaldEagleDamBrk.g09.refpts.66.hdf':'707d518f77a7f9c0ecfbb4f8eb17f5d5446b54c0aedc4d5668d0eca2456e5f8e'})
for name,digest in expected.items():
 path=a.fixture_dir/name;assert hashlib.sha256(path.read_bytes()).hexdigest()==digest
 c=dict(name=name,sha256=digest,version='6.6' if '.66.' in name else '7.0.1',direction_present=False,lines=[])
 with h5py.File(path) as f:
  g=f['Geometry/Reference Points'];attributes=g['Attributes'][()]
  c['geometry']=dict(attributes=freeze(g['Attributes']),points=freeze(g['Points']),features=[dict(name=bytes(row['Name']).decode().rstrip('\x00'),area=bytes(row['SA/2D']).decode().rstrip('\x00'),cell=int(row['Cell Index']),row=i) for i,row in enumerate(attributes)])
  c['series']=[];root='Results/Unsteady/Output/Output Blocks'
  for block,rows in [('Base Output',165),('DSS Hydrograph Output',1441)]:
   group=root+'/'+block+'/Unsteady Time Series'
   if group not in f:continue
   assert f[group+'/Time'].shape==(rows,)
   for prefix in ['Reference Points','2D Flow Areas/BaldEagleCr/Reference Points']:
    ds=f[group+'/'+prefix];names=[bytes(v).decode().rstrip('\x00') for v in ds['Name'][()]]
    global_scope=prefix=='Reference Points'
    want={'Name','Water Surface','Velocity'} if global_scope else {'Name','Water Surface'}
    if global_scope and c['version']=='7.0.1':want.add('Direction')
    assert set(ds)==want,(name,ds.name,set(ds),want)
    geometry_keys=[feat['name']+'|'+feat['area'] if global_scope else feat['name'] for feat in c['geometry']['features']]
    for var,x in ds.items():
     assert var in (['Name','Water Surface','Velocity','Direction'] if global_scope else ['Name','Water Surface'])
     d=freeze(x);d.update(names=names,geometry_rows=[geometry_keys.index(n) for n in names],name_table=ds.name.lstrip('/')+'/Name',role='name_axis' if var=='Name' else ('direction_series' if var=='Direction' else 'time_series'),time_group=None if var=='Name' else group)
     if var!='Name':assert x.shape==(rows,len(names))
     c['series'].append(d)
   for prefix in ['Reference Lines','2D Flow Areas/BaldEagleCr/Reference Lines']:
    ds=f[group+'/'+prefix];global_scope=prefix=='Reference Lines'
    want={'Name','Flow','Velocity','Water Surface','Water Surface Max','Water Surface Min'}
    if c['version']=='6.6':want.add('Water Surface Conveyance')
    elif global_scope:want.update(['Area','Depth Hydraulic','Depth Max','Depth Unit','Friction Slope','RFace Unit','Top Width','Velocity Max','Velocity Unit','Volume Cumulative'])
    assert set(ds)==want,(name,ds.name,set(ds),want)
    names=[bytes(v).decode().rstrip('\x00') for v in ds['Name'][()]]
    assert names==(['RL_Mid|BaldEagleCr'] if global_scope else ['RL_Mid'])
    assert ds['Flow'][()].min() < -950 and ds['Flow'][()].max()==0
    for var,x in ds.items():
     d=freeze(x);d.update(names=names,name_table=ds.name.lstrip('/')+'/Name',role='name_axis' if var=='Name' else 'time_series')
     if var!='Name':assert x.shape==(rows,1)
     c['lines'].append(d)
  if root in f:
   summary=[]
   f[root].visititems(lambda n,x:summary.append(n) if isinstance(x,h5py.Dataset) and '/Reference Points/' in n and '/Summary Output/' in n else None);assert not summary
   base=root+'/Base Output/Unsteady Time Series';cells=[feat['cell'] for feat in c['geometry']['features']]
   c['direction_present']=c['version']=='7.0.1'
  if c['direction_present']:
   x=np.column_stack([f[base+'/2D Flow Areas/BaldEagleCr/Cell Velocity - Velocity X'][:,cell] for cell in cells]);y=np.column_stack([f[base+'/2D Flow Areas/BaldEagleCr/Cell Velocity - Velocity Y'][:,cell] for cell in cells]);d=f[base+'/Reference Points/Direction'][()];v=f[base+'/Reference Points/Velocity'][()];mask=v>0.01
   c['direction_evidence']=dict(nonzero_samples=int(mask.sum()),max_xy_degrees_error=float(np.max(abs(np.degrees(np.arctan2(y,x))[mask]-d[mask]))),max_north_degrees_error=float(np.max(abs(np.degrees(np.arctan2(x,y))[mask]-d[mask]))),max_speed_error=float(np.max(abs(np.hypot(x,y)[mask]-v[mask]))),note='Observed positive-quadrant values only: stored degrees, no inferred wrapping or north-bearing conversion')
 cases.append(c)
a.output.write_text(json.dumps(dict(h5py=h5py.__version__,numpy=np.__version__,cases=cases),indent=2),encoding='utf-8')
