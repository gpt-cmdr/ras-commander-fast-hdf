"""Real HEC-RAS 5.0.7 excerpts for the absent-Location legacy mesh-series rule, from the public
FEMA eBFE Austin-Oyster plan AustinOyster.p03.hdf (2D area 'Perimeter 1').
Source: FEMA estimated Base Flood Elevation (eBFE) program,
https://ebfedata.s3.amazonaws.com/12040205_AustinOyster/12040205_Models.zip
(RAS Model/Hydraulic_Models/RAS_Submittal.zip -> Output.zip -> AustinOyster.p03.hdf).
The source stores Depth/Water Surface/Face Velocity/Face Shear Stress without Location.
Mutations (renames, counts, Location values) are explicit and recorded in provenance."""
from pathlib import Path
import argparse,hashlib,json
import h5py,numpy as np
p=argparse.ArgumentParser();p.add_argument('--fixture',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--area',default='Perimeter 1');p.add_argument('--start-row',type=int,default=70);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
base='Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series'
area=a.area;geo='Geometry/2D Flow Areas/'+area+'/';root=base+'/2D Flow Areas/'+area+'/'
summary=base.replace('Unsteady Time Series','Summary Output')+'/2D Flow Areas/'+area+'/'
CELLS,FACES=3,4 # distinct counts, so a wrong domain cannot pass by coincidence
T0=a.start_row # first copied Time row: every excerpt cell and face is wet there, so values are non-zero
def copy_attr(src,dst,name,target=None):
 """Copy one attribute with its exact stored datatype (bitfields stay bitfields)."""
 aid=src.attrs.get_id(name);tid=aid.get_type();sid=aid.get_space()
 buf=np.empty(aid.shape,dtype=aid.dtype);aid.read(buf,mtype=tid)
 new=h5py.h5a.create(dst.id,(target or name).encode(),tid,sid);new.write(buf,mtype=tid)
def copy(src,dst,path,target=None,data=None,attrs=True):
 x=src[path];y=dst.create_dataset(target or path,data=x[()] if data is None else data,dtype=x.dtype)
 if attrs:
  for k in x.attrs:copy_attr(x,y,k)
 return y
def series(src,dst,name,target=None,rows=2,cols=None):
 n=cols if cols is not None else (CELLS if name in ['Depth','Water Surface'] else FACES)
 return copy(src,dst,root+name,root+(target or name),src[root+name][T0:T0+rows,:n])
with h5py.File(a.fixture,'r') as src:
 hashed=src['Geometry/2D Flow Areas/'+area]
 for mode in ['valid','wrong_count','invalid_location','rows_mismatch','physical_count','physical_count_ok','time_missing','cell_geometry_absent']:
  with h5py.File(a.output/(mode+'.h5'),'w',libver='earliest') as dst:
   for name in ['Time','Time Date Stamp','Time Date Stamp (ms)']:
    if mode=='time_missing' and name=='Time':continue # date stamps only (PR #565 refuses)
    if base+'/'+name in src:copy(src,dst,base+'/'+name,data=src[base+'/'+name][T0:T0+2])
   for name,n in [('Cells Center Coordinate',CELLS),('Faces FacePoint Indexes',FACES),('FacePoints Coordinate',CELLS)]:
    if mode=='cell_geometry_absent' and name=='Cells Center Coordinate':continue
    copy(src,dst,geo+name,data=src[geo+name][:n])
   # Real 5.0.7 HDF5 bitfield attribute (32 bytes, size 1) on the area group and one series.
   copy_attr(hashed,dst['Geometry/2D Flow Areas/'+area],'Property Tables LC Hash')
   if mode=='valid':
    for name in ['Depth','Water Surface','Face Velocity','Face Shear Stress']:series(src,dst,name)
    copy_attr(hashed,dst[root+'Depth'],'Property Tables LC Hash')
    # Real SPACEPAD fixed strings with their exact stored datatype (h5o.copy, not numpy S).
    dst.require_group('Results/Unsteady/Geometry Info')
    h5py.h5o.copy(src.id,b'Results/Unsteady/Geometry Info/2D Area(s)',dst.id,b'Results/Unsteady/Geometry Info/2D Area(s)')
    # The source lacks Face Area / Face Manning's n: renamed real face values, Location still absent.
    series(src,dst,'Face Velocity','Face Area');series(src,dst,'Face Shear Stress',"Face Manning's n")
    # Unknown names never qualify: a distinct quantity, case folding and a prefix/suffix.
    series(src,dst,'Depth','Cell Hydraulic Depth',cols=CELLS);series(src,dst,'Depth','depth',cols=CELLS)
    series(src,dst,'Face Velocity','Face Velocity X',cols=FACES)
    copy(src,dst,root+'Time Step',data=src[root+'Time Step'][T0:T0+2])
    # Summary arrays keep the labelled-summary rule: same exact name, no Location, refused.
    copy(src,dst,root+'Depth',summary+'Depth',src[root+'Depth'][T0:T0+2,:CELLS])
   if mode=='wrong_count':
    series(src,dst,'Depth',cols=CELLS+1);series(src,dst,'Face Velocity',cols=FACES-1)
    series(src,dst,'Water Surface') # unaffected sibling stays qualified
   if mode=='invalid_location':
    series(src,dst,'Depth').attrs['Location']=np.bytes_('Nodes')            # unknown: refuse, no fallback
    series(src,dst,'Water Surface').attrs['Location']=np.bytes_('Faces')    # authoritative: face, 3 != 4 Corrupt
    series(src,dst,'Face Velocity').attrs['Location']=np.array([b'Faces',b'Faces']) # non-scalar: refuse
    series(src,dst,'Face Shear Stress').attrs['Location']=np.int32(1)       # non-text: refuse
    series(src,dst,'Face Velocity','Face Area').attrs['Location']=np.bytes_('Faces') # valid stored
   if mode=='rows_mismatch':
    series(src,dst,'Depth',rows=3);series(src,dst,'Face Velocity')
   if mode=='cell_geometry_absent':
    series(src,dst,'Depth');series(src,dst,'Face Velocity')
    series(src,dst,'Water Surface','Stored Cells').attrs['Location']=np.bytes_('Cells')
   if mode=='time_missing':
    for name in ['Depth','Water Surface','Face Velocity']:series(src,dst,name)
    series(src,dst,'Water Surface','Stored Cells').attrs['Location']=np.bytes_('Cells')
    series(src,dst,'Face Velocity','Stored Faces').attrs['Location']=np.bytes_('Faces')
   if mode.startswith('physical_count'):
    # Real Attributes table (Cell Count far above the 3 excerpt rows); _ok lowers it to 2 (<= 3).
    attrs=src['Geometry/2D Flow Areas/Attributes'][()]
    if mode=='physical_count_ok':attrs['Cell Count']=2
    dst.create_dataset('Geometry/2D Flow Areas/Attributes',data=attrs)
    for name in ['Depth','Water Surface','Face Velocity']:series(src,dst,name)
    series(src,dst,'Water Surface','Stored Cells').attrs['Location']=np.bytes_('Cells')
def sha256(path):
 h=hashlib.sha256()
 with open(path,'rb') as f:
  for b in iter(lambda:f.read(1<<24),b''):h.update(b)
 return h.hexdigest()
with h5py.File(a.fixture,'r') as src:cell_count=int(src['Geometry/2D Flow Areas/Attributes'][()]['Cell Count'][0])
(a.output/'provenance.json').write_text(json.dumps(dict(
 source=dict(name=a.fixture.name,sha256=sha256(a.fixture),area=area,start_row=T0,
  program='FEMA estimated Base Flood Elevation (eBFE)',attribution='Source: FEMA estimated Base Flood Elevation (eBFE) program',
  download_location='https://ebfedata.s3.amazonaws.com/12040205_AustinOyster/12040205_Models.zip',
  path_in_zip='RAS Model/Hydraulic_Models/RAS_Submittal.zip -> Output.zip -> AustinOyster.p03.hdf',licence='Public domain'),
 mutations='two consecutive Time rows from row %d (three for rows_mismatch Depth); first 3 cells/4 faces/3 facepoints; Face Velocity/Face Shear Stress renamed to Face Area/Face Manning\'s n and Depth/Face Velocity to unknown names; Depth/Face Velocity column counts off by one; Location set to Nodes, Faces, a two-element array and an int; real Attributes table (Cell Count %d) and a copy with Cell Count 2; time_missing drops Base Output Time and keeps the date stamps; cell_geometry_absent drops Cells Center Coordinate; real bitfield Property Tables LC Hash and SPACEPAD Geometry Info/2D Area(s) copied with exact datatypes; original literal values retained'%(T0,cell_count),
 files={q.name:sha256(q) for q in sorted(a.output.glob('*.h5'))}),indent=2)+'\n')
