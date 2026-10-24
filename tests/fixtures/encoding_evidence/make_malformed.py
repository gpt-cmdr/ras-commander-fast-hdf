"""Regenerate the byte-mutated metadata-encoding fixtures.

Usage: python -I make_malformed.py ROOT
Writes ROOT/malformed/*.h5 and ROOT/malformed-manifest.json. Pass a scratch
directory; the committed fixtures are frozen and tests verify their hashes.
"""
import h5py,numpy as np,pathlib,struct,json,hashlib,sys
if len(sys.argv)!=2:
 sys.exit('usage: make_malformed.py ROOT')
root=pathlib.Path(sys.argv[1]);root.mkdir(parents=True,exist_ok=True)
out=root/'malformed';out.mkdir(exist_ok=True)
def attr_file(name,value,dtype=None):
 p=out/(name+'.h5')
 with h5py.File(p,'w',libver='earliest') as f:
  f.attrs.create('x',value,dtype=dtype)
 return p
def attr_dt(p):
 b=bytearray(p.read_bytes())
 candidates=[q for q in range(len(b)-16) if b[q:q+4]==b'\x01\x00\x02\x00' and b[q+8:q+10]==b'x\x00']
 assert len(candidates)==1,candidates
 q=candidates[0];ns,ts,ds=struct.unpack_from('<HHH',b,q+2)
 d=q+8+((ns+7)//8*8)
 data=d+((ts+7)//8*8)+((ds+7)//8*8)
 return b,d,data

manifest=[]
def mutate(name,p,fn,expected):
 b,d,v=attr_dt(p);fn(b,d,v);q=out/(name+'.h5');q.write_bytes(b);manifest.append(dict(name=q.name,expected=expected,sha256=hashlib.sha256(b).hexdigest()))
u=attr_file('uint64',np.array([2**53+1,2**64-1],dtype='>u8'));manifest.append(dict(name=u.name,expected='unsigned',sha256=hashlib.sha256(u.read_bytes()).hexdigest()))
i=attr_file('negative',np.array([-1,-(2**63)],dtype='<i8'));manifest.append(dict(name=i.name,expected='signed',sha256=hashlib.sha256(i.read_bytes()).hexdigest()))
f=attr_file('float32',np.array([0.1,-2.5],dtype='>f4'));manifest.append(dict(name=f.name,expected='float',sha256=hashlib.sha256(f.read_bytes()).hexdigest()))
s=attr_file('fixed',np.bytes_('hello'))
manifest.append(dict(name=s.name,expected='text',sha256=hashlib.sha256(s.read_bytes()).hexdigest()))
mutate('integer_width3',u,lambda b,d,v:struct.pack_into('<I',b,d+4,3),'unsupported')
mutate('integer_zero_width',u,lambda b,d,v:struct.pack_into('<I',b,d+4,0),'unsupported')
mutate('datatype_truncated_envelope',u,lambda b,d,v:struct.pack_into('<H',b,d-12,8),'corrupt')
f64=attr_file('float64',np.array([0.1,-2.5],dtype='<f8'));manifest.append(dict(name=f64.name,expected='float64',sha256=hashlib.sha256(f64.read_bytes()).hexdigest()))
mutate('float_offset',f64,lambda b,d,v:struct.pack_into('<H',b,d+8,1),'unsupported')
mutate('float_precision',f64,lambda b,d,v:struct.pack_into('<H',b,d+10,63),'unsupported')
mutate('fixed_utf8',s,lambda b,d,v:(b.__setitem__(d+1,17),b.__setitem__(slice(v,v+5),b'h\xc3\xa9\0\0')),'utf8')
mutate('fixed_nullpad',s,lambda b,d,v:(b.__setitem__(d+1,1),b.__setitem__(slice(v,v+5),b'a\0b\0\0')),'nullpad')
mutate('fixed_spacepad',s,lambda b,d,v:(b.__setitem__(d+1,2),b.__setitem__(slice(v,v+5),b'ab   ')),'spacepad')
mutate('precision' ,u,lambda b,d,v:struct.pack_into('<H',b,d+10,63),'unsupported')
mutate('offset',u,lambda b,d,v:struct.pack_into('<H',b,d+8,1),'unsupported')
mutate('zero_precision',u,lambda b,d,v:struct.pack_into('<H',b,d+10,0),'unsupported')
mutate('integer_padding',u,lambda b,d,v:b.__setitem__(d+1,b[d+1]|2),'unsupported')
mutate('float_layout',f,lambda b,d,v:b.__setitem__(d+12,22),'unsupported')
mutate('float_reserved_high',f,lambda b,d,v:b.__setitem__(d+3,1),'unsupported')
mutate('float_vax',f,lambda b,d,v:b.__setitem__(d+1,b[d+1]|64),'unsupported')
mutate('reserved_padding',s,lambda b,d,v:b.__setitem__(d+1,3),'unsupported')
mutate('reserved_charset',s,lambda b,d,v:b.__setitem__(d+1,32),'unsupported')
v=attr_file('vlen_valid','hello',h5py.string_dtype('utf-8'))
manifest.append(dict(name=v.name,expected='unsupported',sha256=hashlib.sha256(v.read_bytes()).hexdigest()))
mutate('vlen_missing_object',v,lambda b,d,v:struct.pack_into('<I',b,v+12,99),'unsupported')
mutate('vlen_high_index',v,lambda b,d,v:struct.pack_into('<I',b,v+12,65537),'unsupported')
mutate('vlen_length_mismatch',v,lambda b,d,v:struct.pack_into('<I',b,v,999),'unsupported')
b,d,data=attr_dt(v);q=out/'vlen_truncated.h5';q.write_bytes(b[:data+10]);manifest.append(dict(name=q.name,expected='corrupt',sha256=hashlib.sha256(q.read_bytes()).hexdigest()))
# Actual v1 compound member envelopes: scalar versus legacy dimensions.
p=out/'compound.h5'
with h5py.File(p,'w',libver='earliest') as h:
 h.create_dataset('table',data=np.array([(b'abcd',7)],dtype=[('Name','S4'),('Count','<i4')]))
b=bytearray(p.read_bytes());needle=bytes([0x16,2,0,0,8,0,0,0])+b'Name';d=b.index(needle)
manifest.append(dict(name=p.name,expected='compound',sha256=hashlib.sha256(b).hexdigest()))
for name,change,expected in [
 ('zero_record',lambda b:struct.pack_into('<I',b,d+4,0),'corrupt_compound'),
 ('outside_record',lambda b:struct.pack_into('<I',b,d+16,99),'corrupt_compound'),
 ('legacy_dims',lambda b:b.__setitem__(d+20,1),'legacy'),
 ('overlap',lambda b:struct.pack_into('<I',b,d+64,2),'corrupt_compound'),
]:
 c=bytearray(b);change(c);q=out/(name+'.h5');q.write_bytes(c);manifest.append(dict(name=q.name,expected=expected,sha256=hashlib.sha256(c).hexdigest()))
(root/'malformed-manifest.json').write_text(json.dumps(manifest,indent=2))
print(len(manifest),'frozen native-caller cases')
