"""Independent h5py exact-name BC sets plus accepted result descriptor oracle."""
from pathlib import Path
import json,sys,h5py
j=json.loads(Path(sys.argv[1]).read_text())
def text(v):return v.decode('utf-8')
for c in j['cases']:
 with h5py.File(c['source'],'r') as f:
  table=f.get('Geometry/Boundary Condition Lines/Attributes')
  for a in c['areas']:
   geo=[] if table is None else [text(r['Name']) for r in table[:] if text(r['SA-2D'])==a['name']]
   series=[v['name'] for v in a['variables'] if v['family']=='Boundary Conditions']
   a['matched']=sorted(set(geo)&set(series));a['unmatched_geometry']=sorted(set(geo)-set(series));a['unmatched_series']=sorted(set(series)-set(geo))
Path(sys.argv[2]).write_text(json.dumps(j,indent=2))
