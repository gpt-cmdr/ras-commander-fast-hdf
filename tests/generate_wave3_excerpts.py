"""Small real-data excerpts plus one deliberate shape mutation; no HEC-RAS execution."""
import argparse, hashlib, json
from pathlib import Path
import h5py
p=argparse.ArgumentParser();p.add_argument("--fixture-dir",type=Path,required=True);p.add_argument("--output",type=Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
source=a.fixture_dir/"AustinOyster.p01.hdf";base="Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series"
with h5py.File(source,"r") as src:
    for name,bad in [("reference_excerpt.h5",False),("reference_bad_time.h5",True)]:
        with h5py.File(a.output/name,"w",libver="earliest") as dst:
            for key in ["Time","Time Date Stamp","Time Date Stamp (ms)","Reference Lines/Name","Reference Lines/Flow"]:
                path=base+"/"+key
                if path not in src:continue
                ds=src[path];data=ds[:2] if bad and key.endswith("/Flow") else ds[()]
                out=dst.create_dataset(path,data=data,dtype=ds.dtype)
                for k,v in ds.attrs.items():out.attrs[k]=v
pipe_source=a.fixture_dir/"DavisStormSystem.p02.hdf"
with h5py.File(pipe_source,"r") as src,h5py.File(a.output/"pipe_bad_ranges.h5","w",libver="earliest") as dst:
    for name in ["Attributes","Polyline Info","Polyline Parts","Polyline Points"]:
        path="Geometry/Pipe Conduits/"+name
        ds=src[path];data=ds[()]
        if name=="Polyline Info":data[0,0]=2147483647
        dst.create_dataset(path,data=data,dtype=ds.dtype)
provenance=dict(pipe_source_sha256=hashlib.sha256(pipe_source.read_bytes()).hexdigest(),source_name=source.name,source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),files={f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in a.output.glob("*.h5")},mutation="reference_bad_time: Flow first two rows, unchanged full block time axis")
(a.output/"provenance.json").write_text(json.dumps(provenance,indent=2))
