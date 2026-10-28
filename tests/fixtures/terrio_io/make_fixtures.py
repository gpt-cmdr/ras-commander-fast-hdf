"""Generate the small synthetic fixtures of tests/terrio_io.rs and their SHA256 manifest.

Synthetic data only (no model data): a pipe network laid out as HEC-RAS 7.0.1 writes
`Geometry/Pipe Conduits` and `Geometry/Pipe Nodes`, and root `File Type` variants.

    python tests/fixtures/terrio_io/make_fixtures.py
"""

import hashlib
import json
from pathlib import Path

import h5py
import numpy as np

HERE = Path(__file__).resolve().parent


def fixed(text, size, pad=h5py.h5t.STR_NULLPAD):
    """A fixed-length ASCII string type of `size` bytes with the given padding."""
    t = h5py.h5t.C_S1.copy()
    t.set_size(size)
    t.set_strpad(pad)
    return t


def root_attr(f, name, text, pad=h5py.h5t.STR_NULLTERM):
    """A scalar fixed-length root attribute, as HEC-RAS writes them."""
    data = text.encode()
    tid = fixed(text, len(data) + (1 if pad == h5py.h5t.STR_NULLTERM else 0), pad)
    space = h5py.h5s.create(h5py.h5s.SCALAR)
    attr = h5py.h5a.create(f.id, name.encode(), tid, space)
    attr.write(np.array(data, dtype=f"S{tid.get_size()}"), mtype=tid)


def pipes(path):
    with h5py.File(path, "w", libver="earliest") as f:
        root_attr(f, "File Type", "HEC-RAS Geometry")
        root_attr(f, "File Version", "HEC-RAS 7.0.1 June 2026")
        conduits = f.create_group("Geometry/Pipe Conduits")
        attrs = np.array(
            [(b"C1", b"Sys", b"N1", b"N2", 120.5, 1.5, 0.013, 1),
             (b"C2", b"Sys", b"N2", b"N3", 80.25, 2.0, 0.015, 0)],
            dtype=[("Name", "S3"), ("System Name", "S4"), ("US Node", "S3"), ("DS Node", "S3"),
                   ("Conduit Length", "<f8"), ("Rise", "<f4"), ("Manning's n", "<f4"), ("DS Flap Gate", "u1")],
        )
        conduits.create_dataset("Attributes", data=attrs)
        # Two conduits: a 2-point line and a 3-point line, one part each.
        conduits.create_dataset("Polyline Info", data=np.array([[0, 2, 0, 1], [2, 3, 1, 1]], dtype="<i4"))
        conduits.create_dataset("Polyline Parts", data=np.array([[0, 2], [0, 3]], dtype="<i4"))
        conduits.create_dataset("Polyline Points", data=np.array(
            [[0.0, 0.0], [10.0, 0.0], [10.0, 0.0], [15.0, 5.0], [20.0, 5.0]], dtype="<f8"))
        nodes = f.create_group("Geometry/Pipe Nodes")
        node_attrs = np.array(
            [(b"N1", b"Junction", 36.0625), (b"N2", b"Junction", 35.5), (b"N3", b"Outfall", 34.25)],
            dtype=[("Name", "S3"), ("Node Type", "S8"), ("Invert Elevation", "<f4")],
        )
        nodes.create_dataset("Attributes", data=node_attrs)
        nodes.create_dataset("Points", data=np.array([[0.0, 0.0], [10.0, 0.0], [20.0, 5.0]], dtype="<f8"))


def file_types():
    out = {}
    for name, write in {
        "results_nullpad.h5": lambda f: root_attr(f, "File Type", "HEC-RAS Results", h5py.h5t.STR_NULLPAD),
        "terrain.h5": lambda f: root_attr(f, "File Type", "HEC Terrain"),
        "no_file_type.h5": lambda f: root_attr(f, "Units System", "US Customary"),
        "file_type_integer.h5": lambda f: f.attrs.create("File Type", np.int32(7)),
        "file_type_two_strings.h5": lambda f: f.attrs.create("File Type", np.array([b"HEC-RAS Results", b"HEC-RAS Results"])),
    }.items():
        with h5py.File(HERE / name, "w", libver="earliest") as f:
            write(f)
        out[name] = HERE / name
    return out


def main():
    paths = {"pipes.h5": HERE / "pipes.h5"}
    pipes(paths["pipes.h5"])
    paths.update(file_types())
    manifest = {"generator": "make_fixtures.py", "h5py": h5py.version.version, "hdf5": h5py.version.hdf5_version,
                "fixtures": {n: hashlib.sha256(p.read_bytes()).hexdigest() for n, p in sorted(paths.items())}}
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
