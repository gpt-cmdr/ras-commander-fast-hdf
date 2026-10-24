"""Create malformed-rank derivatives of the W3C real-geometry excerpt."""
from pathlib import Path
import hashlib
import json
import h5py
import numpy as np

root = Path(__file__).parent / "fixtures/w3c"
source = root / "summary_valid.h5"
path = ("Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/"
        "2D Flow Areas/Perimeter 1/Renamed Evidence")
records = []
for label, shape in [("scalar", ()), ("rank3", (1, 1, 1)), ("rank4", (1, 1, 1, 1))]:
    dest = root / f"summary_{label}.h5"
    with h5py.File(source, "r") as src, h5py.File(dest, "w") as out:
        for name in src:
            src.copy(name, out)
        attrs = dict(out[path].attrs)
        del out[path]
        ds = out.create_dataset(path, data=np.array(0, dtype="f4").reshape(shape))
        for name, value in attrs.items():
            ds.attrs[name] = value
    records.append(dict(file=dest.name, shape=list(shape), literal_f32=0,
                        sha256=hashlib.sha256(dest.read_bytes()).hexdigest()))
(root / "w3d-provenance.json").write_text(json.dumps(dict(
    sources={str(source.relative_to(root.parent)): hashlib.sha256(source.read_bytes()).hexdigest()},
    purpose="Malformed summary ranks; preserve readable qualified sibling",
    datasets=records), indent=2) + "\n", encoding="utf-8")
