"""Real SPACEPAD string attributes whose elements are NUL fill, copied with their
exact stored datatypes: Baxter.p01 Northside `Storage Area Variables` Variable_Unit (6x2 S32,
units of Surface Area/Volume are 32 NULs; h5py b'') and Weise `Results/Unsteady/Summary`
Transport Function (scalar S80 of NULs; h5py b''). Host values are tiny zero arrays."""
from pathlib import Path
import argparse,hashlib,json
import h5py,numpy as np
p=argparse.ArgumentParser();p.add_argument('--baxter',type=Path,required=True);p.add_argument('--weise',type=Path,required=True)
p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
SA='Results/Steady/Output/Output Blocks/Base Output/Steady Profiles/Storage Areas/Northside/Storage Area Variables'
def copy_attr(src,dst,name):
 aid=src.attrs.get_id(name);tid=aid.get_type();sid=aid.get_space()
 buf=np.empty(aid.shape,dtype=aid.dtype);aid.read(buf,mtype=tid)
 h5py.h5a.create(dst.id,name.encode(),tid,sid).write(buf,mtype=tid)
with h5py.File(a.output/'spacepad.h5','w',libver='earliest') as dst:
 with h5py.File(a.baxter,'r') as src:copy_attr(src[SA],dst.create_dataset('baxter_storage_area_variables',data=np.zeros(1,'<f4')),'Variable_Unit')
 with h5py.File(a.weise,'r') as src:copy_attr(src['Results/Unsteady/Summary'],dst.create_group('weise_summary'),'Transport Function')
(a.output/'provenance.json').write_bytes((json.dumps(dict(
 sources={n.name:hashlib.sha256(n.read_bytes()).hexdigest() for n in [a.baxter,a.weise]},
 attributes={'baxter_storage_area_variables':SA+' @Variable_Unit','weise_summary':'Results/Unsteady/Summary @Transport Function'},
 files={q.name:hashlib.sha256(q.read_bytes()).hexdigest() for q in sorted(a.output.glob('*.h5'))}),indent=2)+'\n').encode())
