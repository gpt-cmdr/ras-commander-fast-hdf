"""Generate malformed HTAB station-elevation evidence from the Wave 2 error fixture."""
from pathlib import Path
import hashlib, json
import h5py
import numpy as np
root = Path(__file__).parent / "fixtures/w3c"
records = []
# The independent HTAB identity probe must not index malformed station evidence.
source_xs = root.parent / "wave2_1d/xs_fields.h5"
for label, shape in [("scalar", ()), ("vector", (1,)), ("rank3", (1, 1, 1))]:
    dest = root / f"htab_station_{label}.h5"
    with h5py.File(source_xs, "r") as src, h5py.File(dest, "w") as out:
        src.copy("Geometry", out)
        g = out["Geometry/Cross Sections"]
        del g["Station Elevation Values"]
        g.create_dataset("Station Elevation Values", data=np.array(0, dtype="f4").reshape(shape))
        g["Station Elevation Info"][...] = [[0, 1], [0, 1]]
        g["Property Tables/XSEC Info"] = np.array([[0, 1], [1, 1], [2, 1]], dtype="i4")
        g["Property Tables/XSEC Value"] = np.zeros((3, 1), dtype="f4")
    records.append(dict(file=dest.name, shape=list(shape), literal_f32=0,
                        sha256=hashlib.sha256(dest.read_bytes()).hexdigest()))
(root / "w3d-xs-provenance.json").write_text(json.dumps(dict(
    sources={str(p.relative_to(root.parent)): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in [source_xs]},
    purpose="Malformed-rank derivatives; preserve readable qualified siblings",
    datasets=records), indent=2) + "\n", encoding="utf-8")
