"""Freeze stored bits via independent h5py. Fixtures never modified. Run with probe.json and output dir."""
from pathlib import Path
import sys,json,hashlib
import h5py,numpy as np
probe=json.loads(Path(sys.argv[1]).read_text());out=Path(sys.argv[2]);out.mkdir(exist_ok=True)
base='Results/Unsteady/Output/Output Blocks/Base Output/'
# API 9: ras-commander PR #565 _LEGACY_MESH_SERIES_DOMAINS, only with Location absent, only at the
# canonical Base Output series path, only for rank-two numeric data (counts asserted below).
LEGACY={'Depth':'cell','Water Surface':'cell','Face Velocity':'face','Face Shear Stress':'face','Face Area':'face',"Face Manning's n":'face'}
cases=[]
for r in probe:
 if not r['exists']:continue
 case={'source':r['path'],'sha256':r['sha256'],'areas':[]};cases.append(case)
 with h5py.File(r['path'],'r') as f:
  for area in f.get('Geometry/2D Flow Areas',{}):
   g=f['Geometry/2D Flow Areas'][area]
   if not isinstance(g,h5py.Group):continue
   counts={k:g[n].shape[0] for k,n in [('cell','Cells Center Coordinate'),('face','Faces FacePoint Indexes'),('facepoint','FacePoints Coordinate')] if n in g}
   a={'name':area,'variables':[]};case['areas'].append(a)
   for family in ['Summary Output','Unsteady Time Series','Boundary Conditions']:
    root=base+(family if family!='Boundary Conditions' else 'Unsteady Time Series')+'/2D Flow Areas/'+area+('/Boundary Conditions' if family=='Boundary Conditions' else '')
    if root not in f:continue
    for name,d in f[root].items():
     if not isinstance(d,h5py.Dataset):continue
     cols=d.shape[-1]
     location=d.attrs.get('Location');location=bytes(location).decode().rstrip('\x00') if isinstance(location,(bytes,np.bytes_)) else None
     domain='other' if family=='Boundary Conditions' else {'Cell':'cell','Cells':'cell','Faces':'face'}.get(location,'other')
     rule='boundary_columns' if family=='Boundary Conditions' else ('stored_location' if domain!='other' else 'unqualified')
     if family=='Unsteady Time Series' and 'Location' not in d.attrs and d.ndim==2 and d.dtype.kind in 'iuf' and d.name=='/'+root+'/'+name and name in LEGACY:domain,rule=LEGACY[name],'legacy_exact_name'
     if family=='Summary Output' and location is None and d.ndim==2 and d.shape[0]==2:
      labels=[bytes(v).decode() for v in d.attrs.get('Rows Variables',[])]
      units=[bytes(v).decode() for v in d.attrs.get('Units',[])]
      if labels==['WSEL','Time'] and len(units)==2 and units[0] in ['ft','m'] and units[1]=='days':domain,rule='cell','labelled_summary'
      # HEC-RAS 5.0: only the two per-row label attributes, with the observed US Customary units.
      elif sorted(d.attrs.keys())==['Row 0:  WSEL','Row 1:  Time'] and [bytes(d.attrs[k]).decode().rstrip(' '+chr(0)) for k in ['Row 0:  WSEL','Row 1:  Time']]==['Feet','Days']:domain,rule='cell','legacy_row_labels'
     assert domain=='other' or (domain in counts and counts[domain]==cols),(d.name,location,d.shape,counts)
     ids=sorted(set(np.linspace(0,max(cols-1,0),min(64,cols),dtype=int).tolist()))
     if len(ids)>1:ids=ids[::-1]+ids[:1] # unsorted and duplicate selection
     rows=[0] if d.ndim==1 else sorted(set([0,d.shape[0]//2,d.shape[0]-1]))
     v={'name':name,'family':family,'path':d.name.lstrip('/'),'shape':list(d.shape),'kind':d.dtype.kind+str(d.dtype.itemsize*8),'domain':domain,'domain_rule':rule,'elements':ids,'rows':rows,'samples':[]}
     refusal = 'schema' if family!='Summary Output' and d.ndim!=2 else ('identity_layout' if family!='Boundary Conditions' and domain=='other' else None)
     v['expected_code']=refusal
     v['chunks']=d.chunks
     v['time_row']=1 if family=='Summary Output' and d.ndim==2 and d.shape[0]==2 and (rule=='legacy_row_labels' or any(k in d.attrs and list(d.attrs[k])[-1]==b'Time' for k in ['Rows Variables','Row Variables'])) else None
     if family!='Summary Output' and d.ndim==2:
      v['slab_end']=min(7,d.shape[0]);v['slab']=np.asarray(d[:v['slab_end']:2,:][:,ids],dtype=d.dtype.newbyteorder('<')).tobytes().hex()
     for row in rows:
      values=d[:] if d.ndim==1 else d[row,:]
      raw=np.asarray(values[ids],dtype=d.dtype.newbyteorder('<')).tobytes()
      v['samples'].append(raw.hex())
     if family=='Summary Output':v['full_sha256']=hashlib.sha256(np.asarray(d[:],dtype=d.dtype.newbyteorder('<')).tobytes()).hexdigest()
     if family=='Unsteady Time Series' and d.ndim==2 and d.dtype.kind=='f':
      window=min(5,d.shape[0]);x=d[:window,:][:,ids]
      v['window']=window
      indices=np.stack([x.argmin(axis=0),x.argmax(axis=0)])
      # Values at first-occurrence indices: numpy min/max can choose a later
      # signed zero, whereas argmin/argmax deliberately choose the first tie.
      v['extrema']=np.asarray(np.take_along_axis(x,indices,axis=0),dtype=d.dtype.newbyteorder('<')).tobytes().hex()
      v['indices']=indices.ravel().tolist()
     a['variables'].append(v)
  case['summary_attrs']=[]
  def attrs(n,g):
   if isinstance(g,h5py.Group) and (n=='Results/Unsteady/Summary' or n.startswith('Results/Unsteady/Summary/Volume Accounting')):
    case['summary_attrs'].extend({'path':n,'name':k,'raw':np.asarray(v).tobytes().hex()} for k,v in g.attrs.items())
  f.visititems(attrs)
(out/'oracle.json').write_text(json.dumps({'h5py':h5py.__version__,'numpy':np.__version__,'cases':cases,'expected_counts':{'supported':sum(v['expected_code'] is None for c in cases for a in c['areas'] for v in a['variables']),'refused':sum(v['expected_code'] is not None for c in cases for a in c['areas'] for v in a['variables'])}},indent=2))
print([(Path(c['source']).name,sum(len(a['variables']) for a in c['areas'])) for c in cases])
with h5py.File(out/'edge.hdf','w') as f:
 g=f.create_group('Geometry/2D Flow Areas')
 g.create_dataset('Attributes',data=np.array([(b'A',3)],dtype=[('Name','S8'),('Cell Count','i4')]))
 a=g.create_group('A')
 for name,n in [('Cells Center Coordinate',3),('Faces FacePoint Indexes',5),('FacePoints Coordinate',4)]:a.create_dataset(name,data=np.zeros((n,2)))
 t=f.create_group(base+'Unsteady Time Series')
 t.create_dataset('Time',data=np.array([0,1/86400,2/86400],dtype='f8'))
 t.create_dataset('Time Date Stamp',data=np.array([b'01JAN2020 00:00:00',b'01JAN2020 00:00:01',b'01JAN2020 00:00:02'],dtype='S24'))
 test=t.create_dataset('2D Flow Areas/A/Test',data=np.array([[1,np.nan,0],[1,2,-0.],[np.nan,3,4]],dtype='f4'),chunks=(3,3));test.attrs['Location']=np.bytes_('Cells')
 t.create_dataset('2D Flow Areas/A/BadRows',data=np.zeros((2,3),dtype='f4'))
 s=f.create_group(base+'Summary Output/2D Flow Areas/A');s.create_dataset('Unmatched',data=np.zeros((1,2),dtype='f4'))
