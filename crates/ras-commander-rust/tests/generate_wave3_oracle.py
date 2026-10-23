"""Freeze h5py values independently of the Rust engine; source files stay read-only.
Usage: python generate_wave3_oracle.py --fixture-dir DIR --output oracle.json
Relocation: set RAS_WAVE3_FIXTURE_DIR to DIR and RAS_WAVE3_ORACLE to oracle.json.
"""
import argparse, hashlib, json
from pathlib import Path
import h5py
import numpy as np

OUTPUT = "Results/Unsteady/Output/Output Blocks"
FAMILIES = ["pipe_geometry", "pipe_results", "reference_line_results", "breach", "plan_metadata"]

def member(path, family):
    if family == "pipe_geometry":
        return any(path.startswith("Geometry/" + k + "/") for k in ["Pipe Networks", "Pipe Conduits", "Pipe Nodes"])
    if family == "plan_metadata":
        return any(path.startswith(k + "/") for k in ["Plan Data", "Results/Summary", "Results/Unsteady/Summary", "Results/Steady/Summary"])
    if family == "breach" and path.startswith("Plan Data/Breach Data/"):
        return True
    if not path.startswith(OUTPUT + "/"):
        return False
    return {"pipe_results": "/Pipe Networks/" in path,
            "reference_line_results": "/Reference Lines/" in path,
            "breach": path.endswith("/Breaching Variables")}.get(family, False)

def attrs(obj):
    out=[]
    for name in sorted(obj.attrs):
        a=np.asarray(obj.attrs[name]);dt=obj.attrs.get_id(name).dtype
        if dt.kind not in "iufS":continue
        if dt.kind in "iu":value=[int(v) for v in a.reshape(-1)];kind="signed" if dt.kind=="i" else "unsigned"
        elif dt.kind=="f":value=[format(int(v),"016x") for v in a.astype("<f8").reshape(-1).view("<u8")];kind="float"
        else:
            try:value=[bytes(v).decode("utf-8").rstrip("\x00") for v in a.reshape(-1)]
            except UnicodeDecodeError:continue
            kind="strings"
        out.append(dict(name=name,kind=kind,values=value))
    return out

def freeze(ds):
    row = dict(path=ds.name.lstrip("/"), shape=list(ds.shape), attrs=sorted(ds.attrs), expected_status="supported",attribute_values=attrs(ds))
    dt = ds.dtype
    if dt.kind in "iuf" and dt.itemsize in [1, 2, 4, 8]:
        if ds.size*dt.itemsize <= 64*1024*1024:
            selections=[tuple(slice(0,n) for n in ds.shape)];coverage="full"
        else:
            # Three separated boxes of up to 1 MiB each, never a tiny 3x4 prefix.
            widths=list(ds.shape)
            while np.prod(widths,dtype=object)*dt.itemsize > 1024*1024:
                axis=max(range(len(widths)),key=lambda i:widths[i]);widths[axis]=(widths[axis]+1)//2
            selections=[tuple(slice((n-w)*position//2,(n-w)*position//2+w) for n,w in zip(ds.shape,widths)) for position in [0,1,2]];coverage="three_boxes_over_64MiB"
        samples=[]
        for selection in selections:
            a=np.asarray(ds[selection] if selection else ds[()])
            samples.append(dict(selection=[[v.start,v.stop,1] for v in selection],selected_shape=list(a.shape),sha256=hashlib.sha256(a.astype(dt.newbyteorder("<"),copy=False).tobytes()).hexdigest()))
        row.update(storage="numeric",kind=dt.kind+str(dt.itemsize),coverage=coverage,**samples[0])
        if len(samples)>1:row["additional_samples"]=samples[1:]
    elif dt.kind == "S":
        try: values=[bytes(x).decode("utf-8").rstrip("\x00") for x in ds[()].reshape(-1)]
        except UnicodeDecodeError: row.update(storage="strings",expected_status="unsupported",expected_code="encoding");return row
        row.update(storage="strings", values=values)
    elif dt.names:
        row["storage"]="compound"
        if any(dt.fields[n][0].subdtype is not None or dt.fields[n][0].kind not in "iufS" for n in dt.names):
            row.update(expected_status="unsupported",expected_code="encoding")
        else:
            a=np.zeros(ds.shape,dtype=dt);ds.id.read(h5py.h5s.ALL,h5py.h5s.ALL,a,mtype=ds.id.get_type())
            row["itemsize"]=dt.itemsize
            row["field_bytes"]=[dict(name=n,offset=dt.fields[n][1],width=dt.fields[n][0].itemsize,sha256=hashlib.sha256(a[n].tobytes()).hexdigest()) for n in dt.names]
            row["columns"]=list(dt.names)
    else:
        row.update(storage="unqualified",expected_status="unsupported",expected_code="encoding")
    return row

def main():
    parser=argparse.ArgumentParser();parser.add_argument("--fixture-dir",type=Path,required=True);parser.add_argument("--output",type=Path,required=True);a=parser.parse_args()
    cases=[]
    for path in sorted(a.fixture_dir.glob("*.hdf")):
        c=dict(name=path.name,sha256=hashlib.sha256(path.read_bytes()).hexdigest(),families={},blocks=[])
        with h5py.File(path,"r") as f:
            datasets={}
            def walk(g,prefix="",depth=0):
                assert depth<=24,"fixture group cycle/depth limit"
                for name,d in g.items():
                    n=prefix+name
                    if isinstance(d,h5py.Dataset):datasets[n]=d
                    else:walk(d,n+"/",depth+1)
            walk(f)
            for family in FAMILIES:
                values=[freeze(d) for n,d in datasets.items() if member(n,family)]
                c["families"][family]=dict(status="supported" if values or family=="plan_metadata" else "unavailable",datasets=values)
                c["families"][family]["coverage"]="observed" if values else "trivially_absent"
                if family=="plan_metadata":
                    groups={"/":attrs(f)}
                    def group_attrs(n,g):
                        if isinstance(g,h5py.Group) and (any(n==p or n.startswith(p+"/") for p in ["Plan Data","Results/Summary","Results/Unsteady/Summary","Results/Steady/Summary"]) or n in ["Results","Results/Unsteady","Results/Steady","Results/Steady/Output"]):groups[n]=attrs(g)
                    f.visititems(group_attrs);c["families"][family]["groups"]=groups;c["families"][family]["coverage"]="observed"
            if OUTPUT in f:
                for block,g in f[OUTPUT].items():
                    if not isinstance(g,h5py.Group) or block=="Base Output":continue
                    c["blocks"].append(dict(name=block,datasets=[freeze(d) for n,d in datasets.items() if n.startswith(g.name.lstrip("/")+"/")]))
            # Independently derive reference and pipe time/identity expectations from h5py.
            for family in ["pipe_results","reference_line_results","breach"]:
                for d in c["families"][family]["datasets"]:
                    n=d["path"];ds=f[n]
                    if n.startswith("Plan Data/"):d["role"]="metadata";continue
                    if n.endswith("/Name"):d["role"]="name_axis";continue
                    if "/Summary Output/" in n:
                        d["role"]="summary_rows"
                        labels=[bytes(v).decode().rstrip("\x00") for v in np.asarray(ds.attrs.get("Rows Variables",[])).reshape(-1)]
                        d["time_row"]=1 if ds.shape[0]==2 and len(labels)>1 and labels[1]=="Time" else None
                    else:
                        group=n.split("/Unsteady Time Series/")[0]+"/Unsteady Time Series"
                        time=f[group+"/Time"]
                        d["time_rows"]=time.shape[0]
                        d["time_group"]=group if ds.shape[0]==time.shape[0] else None
                        # Explicit observed placeholder set, not a general rows==1 semantic rule.
                        placeholder = path.name == "AustinOyster.p01.hdf" and n in {
                            OUTPUT+"/DSS Hydrograph Output/Unsteady Time Series/"+prefix+"Reference Lines/"+v
                            for prefix in ["", "2D Flow Areas/Perimeter 1/"]
                            for v in ["Water Surface Max", "Water Surface Min", "Water Surface Conveyance"]}
                        if placeholder:
                            assert ds.shape == (1,1) and np.array_equal(ds[()],np.zeros((1,1)))
                            d["role"]="one_row_unqualified"
                        else:
                            assert ds.shape[0] == time.shape[0], (path.name,n)
                            d["role"]="breach_progression" if family=="breach" else "time_series"
                        assert ds.shape[0]==time.shape[0] or (family=="reference_line_results" and ds.shape[0]==1)
                    if family=="pipe_results":
                        # Independently freeze the stored Location evidence and its geometry axis.
                        net,var=n.split('/Pipe Networks/')[1].split('/',1)
                        location=ds.attrs.get('Location');location=bytes(location).decode().rstrip('\x00') if isinstance(location,(bytes,np.bytes_)) else None
                        mapping={'Cell Water Surface':('cell','Cell Polygons Info'),'Face Flow':('face','Face Polylines Info'),'Face Velocity':('face','Face Polylines Info'),'Face Water Surface':('face','Face Polylines Info'),'Nodes':('node','Node Indices'),'Pipe Links':('conduit','Conduit Indices')}
                        if location not in mapping:
                            d.update(expected_status='unsupported',expected_code='identity_layout',generic_numeric_fallback=True);continue
                        domain,table=mapping[location];identity="Geometry/Pipe Networks/"+net+"/"+table
                        d.update(domain=domain,identity_table=identity,location_count=f[identity].shape[0],location=location)
                        assert ds.shape[1]==f[identity].shape[0]
                        if domain in ['node','conduit']:d['id_values']=freeze(f[identity])
                    if family=="reference_line_results":
                        names=f[n.rsplit("/",1)[0]+"/Name"]
                        assert names.shape==(ds.shape[1],)
                        d["names"]=[bytes(v).decode().rstrip("\x00") for v in names[()]]
                    if family=="breach":
                        assert ds.attrs["Variable_Unit"].shape==(ds.shape[1],2)
        cases.append(c)
        print(path.name,sum(len(v["datasets"]) for v in c["families"].values()),flush=True)
    assert cases, "no qualification fixtures found"
    a.output.write_text(json.dumps(dict(h5py=h5py.__version__,numpy=np.__version__,cases=cases),indent=2),encoding="utf-8")
if __name__=="__main__":main()
