"""Reproduce tiny real-data classification and malformed sediment excerpts."""
import argparse,hashlib,json
from pathlib import Path
import h5py
p=argparse.ArgumentParser();p.add_argument("--fixture-dir",type=Path,required=True);p.add_argument("--output",type=Path,required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
src=a.fixture_dir/"00-LandCover.hdf"
with h5py.File(src,"r") as f,h5py.File(a.output/"ambiguous_classes.h5","w",libver="earliest") as g:
    for k,v in f.attrs.items():g.attrs[k]=v
    v=f["Variables"][:3];v["Name"][0]=b"duplicate";v["Name"][1]=b"duplicate";v["Name"][2]=b"unmatched"
    m=f["Raster Map"][:2];m["Name"][0]=b"duplicate";m["Name"][1]=b"orphan"
    g.create_dataset("Variables",data=v);g.create_dataset("Raster Map",data=m)
sed=a.fixture_dir/"sediment-2d-Weise.p01.hdf";root="Results/Unsteady/Output/Output Blocks/Sediment Bed/Unsteady Time Series"
with h5py.File(sed,"r") as f,h5py.File(a.output/"scalar_sediment_time.h5","w",libver="earliest") as g:
    g.create_dataset(root+"/Time",data=f[root+"/Time"][0])
    choices=[]
    f[root].visititems(lambda n,d:choices.append(n) if isinstance(d,h5py.Dataset) and len(d.shape)==2 and n.startswith("2D Flow Areas/") else None)
    name=choices[0];g.create_dataset(root+"/"+name,data=f[root+"/"+name][:2,:1])
(a.output/"provenance.json").write_text(json.dumps(dict(sources={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [src,sed]},files={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in a.output.glob("*.h5")},mutations=["duplicate/orphan/unmatched class names","scalar Time with a real first2x1 sediment result excerpt"]),indent=2),encoding="utf-8")
