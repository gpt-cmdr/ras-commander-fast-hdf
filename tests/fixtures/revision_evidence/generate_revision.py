"""Frozen synthetic HDF boundaries; run python -I this_script.py (writes revision/ next to the script)."""
import h5py,numpy as np,pathlib,struct,json,hashlib
root=pathlib.Path(__file__).parent;out=root/'revision';out.mkdir(exist_ok=True)
cases=[]
def save(name,b,**expect):
 p=out/(name+'.h5');p.write_bytes(b);cases.append(dict(name=p.name,sha256=hashlib.sha256(b).hexdigest(),**expect));return p
def attr(name,data,dtype=None,libver='earliest',**expect):
 p=out/(name+'.h5')
 with h5py.File(p,'w',libver=libver) as f:f.attrs.create('x',data,dtype=dtype)
 return save(name,p.read_bytes(),**expect)
def parts(p):
 b=bytearray(p.read_bytes());q=b.index(b'\x01\x00\x02\x00',96);assert b[q+8:q+10]==b'x\0'
 ns,ts,ds=struct.unpack_from('<HHH',b,q+2);d=q+8+(ns+7)//8*8;v=d+(ts+7)//8*8+(ds+7)//8*8
 return b,q,d,v,ts,ds
def mutate(name,p,fn,**expect):
 b,q,d,v,ts,ds=parts(p);fn(b,q,d,v,ts,ds);return save(name,b,**expect)
for width in [1,2,4]:
 attr('signed'+str(width),np.array([-1,-(2**(width*8-1))],dtype='<i'+str(width)),value=[-1,-(2**(width*8-1))],kind='signed')
 attr('unsigned'+str(width),np.array([0,2**(width*8)-1],dtype='<u'+str(width)),value=[0,2**(width*8)-1],kind='unsigned')
attr('float32le',np.array([1.5,-2.5],dtype='<f4'),value=[1.5,-2.5],kind='float')
f=attr('float64be',np.array([1.5,-2.5],dtype='>f8'),value=[1.5,-2.5],kind='float')
for name,offset,val in [('sign',2,62),('exponent_location',12,51),('exponent_size',13,10),('mantissa_location',14,1),('mantissa_size',15,51),('bias',16,0),('normalization',1,1)]:
 mutate('float_'+name,f,lambda b,q,d,v,ts,ds,o=offset,x=val:b.__setitem__(d+o,x),kind='unsupported',reason='non-IEEE')
s=attr('text_base',np.bytes_('hello'),kind='text',value=['hello'])
mutate('nullterm',s,lambda b,q,d,v,ts,ds:(b.__setitem__(d+1,0),b.__setitem__(slice(v,v+5),b'a\0bxx')),kind='text',value=['a'])
mutate('zero_width',s,lambda b,q,d,v,ts,ds:struct.pack_into('<I',b,d+4,0),kind='unsupported',reason='zero-width')
mutate('utf8_invalid',s,lambda b,q,d,v,ts,ds:(b.__setitem__(d+1,17),b.__setitem__(v,255)),kind='unsupported',reason='invalid UTF-8',raw_prefix=[255])
mutate('ascii_invalid',s,lambda b,q,d,v,ts,ds:b.__setitem__(v,233),kind='unsupported',reason='non-ASCII',raw_prefix=[233])
# Demonstrate an unrelated bad root attribute does not abort valid Projection.
p=out/'crs_ascii.h5'
with h5py.File(p,'w',libver='earliest') as h:
 h.attrs.create('Title',np.bytes_(b'\xe9bad'));h.attrs.create('Projection',np.bytes_('PROJCS["x",UNIT["metre",1]]'))
save('crs_ascii',p.read_bytes(),kind='crs_ascii')
# Real latest-library v3 attribute envelope with standard fixed-point type.
attr('latest_attribute3',np.array([-1,-32768],dtype='<i2'),libver='latest',kind='signed',value=[-1,-32768])
# Re-encode unpadded v2/v3 attribute envelopes in old checksum-free object header.
for version in [2,3]:
 b,q,d,v,ts,ds=parts(s);ds_at=d+(ts+7)//8*8
 payload=bytes([version,0])+struct.pack('<HHH',2,ts,ds)+(b'\0' if version==3 else b'')+b'x\0'+b[d:d+ts]+b[ds_at:ds_at+ds]+b[v:v+5]
 size=struct.unpack_from('<H',b,q-6)[0];assert len(payload)<=size
 b[q:q+size]=payload+bytes(size-len(payload))
 save('attribute'+str(version),b,kind='text',value=['hello'])
 b[q+1]=1;save('attribute'+str(version)+'_shared',b,kind='error',reason='shared datatype/dataspace',error='Unsupported')
 b[q+1]=128;save('attribute'+str(version)+'_reserved',b,kind='error',reason='reserved flags',error='Corrupt')
# Truncate the declared attribute payload, retaining the whole file and heap.
vlen=attr('vlen_valid','hello',h5py.string_dtype('utf-8'),kind='unsupported',reason='nested/array/vlen/reference')
b,q,d,v,ts,ds=parts(vlen);struct.pack_into('<H',b,q-6,v-q+10);save('vlen_reference_short_envelope',b,kind='error',error='Corrupt',reason='')
# Actual compound datasets; v1 dimensions are at member+12..+28.
p=out/'compound1.h5'
with h5py.File(p,'w',libver='earliest') as h:h.create_dataset('table',data=np.array([(b'abcd',7)],dtype=[('Name','S4'),('Count','<i4')]))
b=bytearray(p.read_bytes());d=b.index(bytes([0x16,2,0,0,8,0,0,0])+b'Name');save('compound1',b,kind='compound')
for name,dims,extent,off in [('legacy_overlap',1,2,4),('legacy_outside',1,3,4),('legacy_dimensions5',5,1,4)]:
 c=bytearray(b);c[d+20]=dims;struct.pack_into('<I',c,d+32,extent);struct.pack_into('<I',c,d+64,off);save(name,c,kind='compound_error',reason='dimensionality' if dims==5 else ('outside record' if extent==3 else 'overlapping'))
# Lossless byte envelopes for v2/v3 compound message variants in the same file.
for version in [2,3]:
 name1=b'Name\0'+(bytes(3) if version==2 else b'');name2=b'Count\0'+(bytes(2) if version==2 else b'')
 env=bytes([(version<<4)|6,2,0,0])+struct.pack('<I',8)+name1+(struct.pack('<I',0) if version==2 else b'\0')+b[d+48:d+56]+name2+(struct.pack('<I',4) if version==2 else b'\4')+b[d+96:d+108]
 c=bytearray(b);c[d:d+len(env)]=env;save('compound'+str(version),c,kind='compound')
for name,dtype in [('nested',np.dtype([('Nested',[('n','<i4')])])),('array_member',np.dtype([('Array','<i4',(2,))]))]:
 p=out/(name+'.h5')
 with h5py.File(p,'w',libver='earliest') as h:h.create_dataset('table',data=np.zeros(1,dtype=dtype))
 save(name,p.read_bytes(),kind='compound_unsupported')
(root/'revision-manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
print(len(cases),'revision cases; h5py',h5py.__version__,'HDF5',h5py.version.hdf5_version)
