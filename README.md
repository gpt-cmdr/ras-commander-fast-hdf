# ras-commander-fast-hdf

A native (Rust) reader for HEC-RAS HDF files, and the optional Python extension
that lets [ras-commander](https://github.com/gpt-cmdr/ras-commander) use it.

With the extension installed, ras-commander reads plan results, geometry,
terrain and land-cover HDF files through this engine instead of h5py. The
public results are identical (same values, dtypes, columns, index, CRS,
attributes and exceptions), and large reads are faster. Without it,
ras-commander works exactly as before.

```bash
pip install ras-commander ras-commander-fast-hdf
```

```python
from ras_commander import fast_hdf
print(fast_hdf.available())   # True: ras-commander now reads natively where it can
```

User documentation (engine modes, what is qualified, performance, limitations)
is on the ras-commander site: **[Native Rust HDF engine](https://rascommander.info/ras/native-engine/)**.
This repository's [docs](docs/README.md) cover the engine itself: architecture,
the Python module contract, building, testing and releasing.

## What is in this repository

| Path | What it is |
|---|---|
| [`crates/ras-commander-rust`](crates/ras-commander-rust) | The engine: a Rust library with no Python dependency. It reads HEC-RAS project and plan files and the HDF5 files HEC-RAS writes (numeric, compound and string datasets, attributes, mesh topology, 1D and 2D results, structures, terrain modifications), and refuses anything it cannot read exactly. |
| [`python`](python) | The Python extension `ras_commander_fast_hdf` (PyO3 + maturin), built as the `ras-commander-fast-hdf` wheel. It exposes the engine to ras-commander, plus native mesh-polygon assembly, fluvial/pluvial kernels and PMTiles/vector exports. |
| [`vendor`](vendor) | A vendored, locally patched copy of the pure-Rust `hdf5-reader` and `hdf5-core` crates (MIT/Apache-2.0). See [`vendor/LOCAL_PATCHES.md`](vendor/LOCAL_PATCHES.md). |
| [`docs`](docs/README.md) | Developer documentation. |

## How it fits together

```
your code ──> ras-commander (Hdf* classes, unchanged API)
                  │  RAS_COMMANDER_FAST_HDF = auto | h5py | native
                  ├──> h5py + HDF5 C library        (reference path)
                  └──> ras_commander_fast_hdf       (this repo, python/)
                           └──> ras-commander-rust  (this repo, crates/)
                                    └──> hdf5-reader (vendor/, pure Rust)
```

In the default `auto` mode, ras-commander tries the native read first and
falls back to h5py whenever the engine refuses (an unsupported layout, a read
over budget, damaged data), so results never differ. `native` mode raises
instead of falling back, which is how qualification proves a workflow really
ran natively. See [Architecture](docs/architecture.md).

## Status

- **Version 0.1.0**, Python extension `API_VERSION` 11. ras-commander accepts
  only the exact `API_VERSION` it was built against; 11 is used by the first
  ras-commander release after 0.106.1.
- **Platforms:** CPython 3.10 or later (stable ABI). Windows x64 and Linux x64
  (`manylinux_2_28`) are qualified. macOS arm64 builds in CI but has not been
  qualified against the HEC-RAS corpus.
- **Proven identical** to h5py on 201 public ras-commander method cases across
  137 HEC-RAS 5.x–7.x result and geometry files: 3,800 method × file pairs in
  three modes, plus 1,428 calls replayed from the example notebooks. Methods
  the corpus has not exercised natively stay on h5py in `auto` mode.
- **Speed** (ras-commander public calls, warm OS cache): 2D cell polygons about
  12× faster (a 340,000-cell mesh drops from 17.2 s to 0.9 s), mesh time series
  3–5×, all calls 1.3× on repeat. Small single reads are a few milliseconds
  slower because of a fixed per-read setup cost.
- **Not supported natively** (ras-commander uses h5py for these in `auto`):
  cross-section tables written before HEC-RAS 5.0.7, and names containing
  non-ASCII characters.

## Building from source

You need Rust 1.88 or later and Python 3.10 or later. On Windows, install the
Visual Studio C++ Build Tools.

```bash
pip install maturin
maturin build --release --locked --manifest-path python/Cargo.toml \
    --features extension-module,exports-all,exports-pmtiles
pip install target/wheels/ras_commander_fast_hdf-*.whl

cargo test --locked --all-features                       # engine
cargo test --locked --manifest-path python/Cargo.toml    # Python bridge (Rust side)
```

See [Building](docs/building.md) and [Testing](docs/testing.md).

## Contributing

Issues and pull requests are welcome. Engine changes must keep reading
HEC-RAS files exactly or refuse them; they are qualified against h5py through
ras-commander's equivalence suites (see [Testing](docs/testing.md)). Agents and
contributors: start with [AGENTS.md](AGENTS.md).

## Licence

MIT; see [LICENSE](LICENSE). The wheel bundles the licences of every linked
crate in [`python/THIRD-PARTY-NOTICES.md`](python/THIRD-PARTY-NOTICES.md).
HEC-RAS is software of the U.S. Army Corps of Engineers Hydrologic Engineering
Center; this project is not affiliated with or endorsed by USACE.
