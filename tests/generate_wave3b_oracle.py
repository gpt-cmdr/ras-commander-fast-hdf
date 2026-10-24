"""Independent h5py oracle for W3B; basename+SHA256 relocation, no native calls."""
from generate_wave3_oracle import freeze as scalar_freeze,attrs
import argparse,hashlib,json
from pathlib import Path
import h5py
import numpy as np
def freeze(ds):
    row=scalar_freeze(ds)
    if ds.id.get_type().get_class()==h5py.h5t.BITFIELD:
        row.update(storage="bitfield_bytes",sha256=hashlib.sha256(ds[()].tobytes()).hexdigest())
        row.pop("selection",None)
    if row.get("storage")=="numeric" and ds.size*ds.dtype.itemsize<=64*1024*1024:
        a=ds[()];row.update(sha256=hashlib.sha256(a.astype(ds.dtype.newbyteorder("<"),copy=False).tobytes()).hexdigest(),selected_shape=list(ds.shape),selection=[[0,n,1] for n in ds.shape],coverage="full")
    elif row.get("storage")=="numeric":row["coverage"]="prefix_over_64MiB"
    n=ds.name
    if "/Sediment Time Series/" in n and "/Cross Section SE/" not in n and row["storage"]=="numeric" and n.rsplit("/",1)[1] not in {"Time","Time Step","Time Date Stamp","Time Date Stamp (ms)"}:
        group=n.split("/Sediment Time Series/")[0]+"/Sediment Time Series"
        if len(ds.shape)<2 or (ds.shape[0]==1 and ds.file[group+"/Time"].shape[0]>1):
            row.update(expected_status="unsupported",expected_code="schema")
    static_names={"Cell Initial Bed Elevation","Subcell Area","Subcell Initial Bed Elevation","Subcell Initial Hydraulic Bed Elevations","Subface Initial Bed Elevation","Subface Length"}
    if "/Unsteady Time Series/" in n and "/Sediment" in n and row["storage"]=="numeric" and not n.rsplit("/",1)[1].startswith("Time") and n.rsplit("/",1)[1] not in static_names:
        group=n.split("/Unsteady Time Series/")[0]+"/Unsteady Time Series"
        if len(ds.shape)<2 or ds.shape[0]!=ds.file[group+"/Time"].shape[0]:row.update(expected_status="unsupported",expected_code="schema")
    if ds.dtype.names and row["expected_status"]=="supported":
        # H5Dread does not define compound padding; compare declared member bytes,
        # with an exact file memory datatype to prevent string-padding conversion.
        a=np.zeros(ds.shape,dtype=ds.dtype);ds.id.read(h5py.h5s.ALL,h5py.h5s.ALL,a,mtype=ds.id.get_type())
        row["itemsize"]=ds.dtype.itemsize
        row["field_bytes"]=[dict(name=n,offset=ds.dtype.fields[n][1],width=ds.dtype.fields[n][0].itemsize,sha256=hashlib.sha256(a[n].tobytes()).hexdigest()) for n in ds.dtype.names]
    return row
FAMILIES=["land_cover","infiltration","terrain","sediment","encroachments","event_conditions","steady_output"]
ROOTS={"land_cover":["Geometry/Land Cover (Manning's n)"],"infiltration":["Geometry/Infiltration"],"terrain":["Terrain"],"sediment":["Sediment","Results/Unsteady/Output/Output Blocks/Sediment","Results/Unsteady/Output/Output Blocks/Sediment SE","Geometry/Sediment","Plan Data/Sediment","Event Conditions/Sediment","Results/Sediment","Results/Unsteady/Output/Output Blocks/Sediment Bed","Results/Unsteady/Output/Output Blocks/Sediment Transport"],"encroachments":["Plan Data/Encroachments","Plan Data/Encroachment Data","Event Conditions/Steady Flow/Encroachment Data","Plan Data/Plan Parameters/Encroachment","Geometry/Cross Sections/Encroachment Stations","Results/Steady/Output/Cross Sections/Encroachment Stations","Results/Steady/Output/Output Blocks/Base Output/Steady Profiles","Geometry/Cross Sections/Attributes"],"event_conditions":["Event Conditions"],"steady_output":["Results/Steady/Output"]}
def selected(f,n,family):
    lc=f.attrs.get("LC Type",b"");lc=lc.decode() if isinstance(lc,bytes) else str(lc)
    flat=all(k in f for k in ["IDs","Names","ManningsN"])
    side=(family=="land_cover" and (flat or lc in ["LandCover","Soils"])) or (family=="infiltration" and lc.startswith("Infiltration")) or (family=="sediment" and lc=="SedimentBedMaterial")
    if side:return True
    if any(n==r or n.startswith(r+"/") for r in ROOTS[family]):return True
    if family=="sediment" and n.startswith("Geometry/2D Flow Areas/") and ("Sediment" in f or "Results/Sediment" in f or any("Sediment" in k for k in f.get("Results/Unsteady/Output/Output Blocks",{})) or any("Sediment Bed Material" in f["Geometry/2D Flow Areas/"+k] for k in f.get("Geometry/2D Flow Areas",{}) if isinstance(f["Geometry/2D Flow Areas/"+k],h5py.Group))):
        if n.endswith("/Cells Center Coordinate") or n.endswith("/Cells Surface Area") or n=="Geometry/2D Flow Areas/Attributes":return True
    if n.startswith("Geometry/2D Flow Areas/"):
        rest=n[len("Geometry/2D Flow Areas/"):].split("/",1)
        if len(rest)==2:
            part=rest[1].split("/")[0]
            return {"land_cover":"Manning" in part or part=="Land Cover","infiltration":"Infiltration" in part,"sediment":"Sediment" in part}.get(family,False)
    return False

def main():
    p=argparse.ArgumentParser();p.add_argument("--fixture-dir",type=Path,required=True);p.add_argument("--output",type=Path,required=True);a=p.parse_args();cases=[]
    for path in sorted(a.fixture_dir.glob("*.hdf")):
        c=dict(name=path.name,sha256=hashlib.sha256(path.read_bytes()).hexdigest(),families={})
        with h5py.File(path,"r") as f:
            datasets={};groups={}
            def walk(g,prefix="",depth=0):
                assert depth<=24
                for k,v in g.items():
                    n=prefix+k
                    if isinstance(v,h5py.Dataset):datasets[n]=v
                    else:groups[n]=v;walk(v,n+"/",depth+1)
            walk(f)
            for family in FAMILIES:
                ds=[freeze(v) for n,v in datasets.items() if selected(f,n,family)]
                gs={n:attrs(v) for n,v in groups.items() if selected(f,n,family)}
                if ds or gs:gs["/"]=attrs(f)
                c["families"][family]=dict(status="supported" if ds or gs else "unavailable",datasets=ds,groups=gs)
            for family in ["land_cover","infiltration"]:
                if not any(selected(f,k,family) for k in ["IDs","Variables","Raster Map"]):continue
                links=[]
                if all(k in f for k in ["IDs","Names","ManningsN"]):
                    links=[dict(name=bytes(n).decode().rstrip("\x00"),pixel_id=int(i),rows=[j],status="supported") for j,(i,n) in enumerate(zip(f["IDs"][()],f["Names"][()]))];unmatched=[]
                elif "Variables" in f or "Raster Map" in f:
                    vnames=[bytes(n).decode().rstrip("\x00") for n in f["Variables"][()]["Name"]] if "Variables" in f else []
                    mapnames=[bytes(n).decode().rstrip("\x00") for n in f["Raster Map"][()]["Name"]] if "Raster Map" in f else vnames
                    ids=[int(i) for i in f["Raster Map"][()]["ID"]] if "Raster Map" in f else [None]*len(mapnames)
                    matched=set()
                    for name,pixel in zip(mapnames,ids):
                        rows=[j for j,n in enumerate(vnames) if n==name];matched.update(rows)
                        links.append(dict(name=name,pixel_id=pixel,rows=rows,status="supported" if len(rows)==1 else ("unsupported" if rows else "unavailable")))
                    unmatched=[j for j in range(len(vnames)) if j not in matched]
                else:continue
                c["families"][family]["classification"]=dict(links=links,unmatched=unmatched)
        cases.append(c);print(path.name,sum(len(v["datasets"]) for v in c["families"].values()),flush=True)
    assert cases, "no qualification fixtures found"
    a.output.write_text(json.dumps(dict(cases=cases),indent=2),encoding="utf-8")
if __name__=="__main__":main()
