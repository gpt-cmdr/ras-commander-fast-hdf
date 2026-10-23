"""Small synthetic storage regressions; originals from h5py, then explicit byte mutations."""
from pathlib import Path
import h5py,numpy as np,json,hashlib
out=Path(__file__).parent
with h5py.File(out/'dense_bad.h5','w',libver='latest') as f:
 g=f.create_group('G');g.create_dataset('ok',data=np.arange(10,dtype='f4'));d=g.create_dataset('bad',data=np.arange(10,dtype='f4'))
 for k in range(30):d.attrs[f'a{k:02d}']=np.float64(k)
p=out/'dense_bad.h5';b=p.read_bytes();assert b.count(b'FRHP')==1;p.write_bytes(b.replace(b'FRHP',b'XXXX'))
with h5py.File(out/'parallel.h5','w',libver='earliest') as f:
 x=np.arange(400*500,dtype='f8').reshape(400,500)/7;x[0,0]=-0.;x[1,1]=np.nan
 f.create_dataset('data',data=x,chunks=(37,61),compression='gzip',shuffle=True)
with h5py.File(out/'header_base.h5','w',libver='earliest') as f:
 d=f.create_dataset('data',data=np.arange(8,dtype='f4'));addr=h5py.h5o.get_info(d.id).addr
vendor=out.parents[4]/'vendor/hdf5-reader/testdata';vendor.mkdir(exist_ok=True)
with h5py.File(vendor/'empty_heap_past_eof.h5','w',libver='earliest'):pass
p=vendor/'empty_heap_past_eof.h5';b=bytearray(p.read_bytes());i=b.index(b'HEAP');b[i+24:i+32]=(len(b)+4096).to_bytes(8,'little');p.write_bytes(b)
(out/'manifest.json').write_text(json.dumps({'header_address':addr,'h5py':h5py.__version__,'files':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in out.glob('*.h5')},'empty_heap_sha256':hashlib.sha256(p.read_bytes()).hexdigest()},indent=2))
