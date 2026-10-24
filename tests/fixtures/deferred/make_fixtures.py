"""Supplemental deferred inventory cases; python -I make_fixtures.py OUTPUT_DIR."""
from pathlib import Path
import sys,json,hashlib
import h5py,numpy as np
out=Path(sys.argv[1]);out.mkdir(parents=True,exist_ok=True)
for case in (sys.argv[2:] or ['empty','unsteady-missing','wrong-root','missing','extra','duplicate','blocks','steady','steady-unknown','wrong-block','wrong-family','numeric']):
 with h5py.File(out/(case+'.h5'),'w',libver='earliest') as f:
  if case=='numeric':
   data=np.arange(12,dtype='<f8').reshape(3,4)
   f['contiguous']=data;f.create_dataset('chunked',data=data,chunks=(2,4));continue
  g=f.create_group('Geometry/2D Flow Areas')
  names=[(b'A',2),(b'B',1)] if case=='missing' else [(b'A',2),(b'A',2)] if case=='duplicate' else [(b'A',2)]
  if case=='empty':names=[]
  g['Attributes']=np.array(names,dtype=[('Name','S8'),('Cell Count','<i4')])
  if case=='empty':continue
  if case=='unsteady-missing':f.create_group('Results/Unsteady')
  if case=='wrong-root':f['Results/Unsteady/Output/Output Blocks']=np.int32(1)
  g['A/Cells Center Coordinate']=np.zeros((3,2),dtype='<f8')
  if case=='extra':g.create_group('Extra')
  if case=='blocks':
   for block in ['Base Output','Sediment Bed']:
    f[f'Results/Unsteady/Output/Output Blocks/{block}/Unsteady Time Series/2D Flow Areas/A/Flow']=np.ones((2,3),dtype='<f4')
  if case=='steady':f['Results/Steady/Output/Output Blocks/Base Output/Steady Profiles/2D Flow Areas/A/Flow']=np.ones((2,3),dtype='<f8')
  if case=='steady-unknown':f.create_group('Results/Steady')
  if case=='wrong-block':f['Results/Unsteady/Output/Output Blocks/Broken']=np.int32(1)
  if case=='wrong-family':f['Results/Steady']=np.int32(1)
(out/'manifest.json').write_text(json.dumps({p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.glob('*.h5'))},indent=2)+'\n')
