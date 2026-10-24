"""Independent h5py oracle for API 9 raw payloads over real fixtures (never modified):
every HDF5 bitfield attribute (h5py uint array bytes) and every fixed-length string dataset up to
1 MiB (declared padding plus h5py `S<width>` bytes, which HDF5 converts to NULLPAD on read).
Usage: raw_payload_oracle.py probe.json oracle.json   (probe: [{exists,path,sha256}])"""
from pathlib import Path
import sys,json
import h5py,numpy as np
probe=json.loads(Path(sys.argv[1]).read_text());cases=[]
for r in probe:
 if not r['exists']:continue
 case={'source':r['path'],'sha256':r['sha256'],'bitfields':[],'strings':[],'skipped':[]};cases.append(case)
 with h5py.File(r['path'],'r') as f:
  def visit(name,o):
   for k in o.attrs:
    try:
     aid=o.attrs.get_id(k);t=aid.get_type()
     if t.get_class()!=h5py.h5t.BITFIELD:continue
     v=np.asarray(o.attrs[k])
     case['bitfields'].append({'path':name,'kind':'group' if isinstance(o,h5py.Group) else 'dataset','name':k,'size':t.get_size(),'big':t.get_order()==h5py.h5t.ORDER_BE,'shape':list(aid.shape),'dtype':v.dtype.str,'h5py':v.astype(v.dtype.newbyteorder('<') if t.get_order()==h5py.h5t.ORDER_LE else v.dtype).tobytes().hex()})
    except Exception as e:case['skipped'].append({'path':name,'name':k,'reason':repr(e)})
   if isinstance(o,h5py.Dataset):
    try:
     if o.dtype.kind!='S':return
     t=o.id.get_type()
     if t.is_variable_str() or o.size*o.dtype.itemsize>1<<20:return
     case['strings'].append({'path':name,'width':o.dtype.itemsize,'padding':t.get_strpad(),'utf8':t.get_cset()==h5py.h5t.CSET_UTF8,'shape':list(o.shape),'h5py':np.asarray(o[...]).tobytes().hex()})
    except Exception as e:case['skipped'].append({'path':name,'reason':repr(e)})
  f.visititems(visit)
  visit('/',f)
counts={'bitfields':sum(len(c['bitfields']) for c in cases),'strings':sum(len(c['strings']) for c in cases),'skipped':sum(len(c['skipped']) for c in cases)}
Path(sys.argv[2]).write_text(json.dumps({'h5py':h5py.__version__,'numpy':np.__version__,'counts':counts,'cases':cases},indent=1))
print(counts,[(Path(c['source']).name,len(c['bitfields']),len(c['strings']),len(c['skipped'])) for c in cases])
