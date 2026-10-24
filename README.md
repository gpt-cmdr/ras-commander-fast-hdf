# ras-commander-fast-hdf

ras-commander's default HDF engine: a native (Rust) HEC-RAS HDF reader and its Python extension for [ras-commander](https://github.com/gpt-cmdr/ras-commander).

[![CI](https://github.com/gpt-cmdr/ras-commander-fast-hdf/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/gpt-cmdr/ras-commander-fast-hdf/actions/workflows/ci.yml)
[![PyPI version](https://img.shields.io/pypi/v/ras-commander-fast-hdf)](https://pypi.org/project/ras-commander-fast-hdf/)
[![Python versions](https://img.shields.io/pypi/pyversions/ras-commander-fast-hdf)](https://pypi.org/project/ras-commander-fast-hdf/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

This repository contains a native (Rust) reader for HEC-RAS HDF files and the
Python extension through which ras-commander uses it. It is ras-commander's
default HDF engine: ras-commander 0.107.1 and later install it automatically
on Windows x64, Linux x64 and macOS arm64.

ras-commander reads plan results, geometry, terrain and land-cover HDF files
through this engine. The public results are identical to the h5py reference
implementation (same values, dtypes, columns, index, CRS, attributes and
exceptions), and large reads are faster. That makes it practical to extract
results from many large 2D models for flood science, research and new kinds of
analysis. h5py remains the fallback: in the default `auto` mode any read the
engine refuses uses h5py, and on platforms without a wheel ras-commander uses
h5py for every read.

| Documentation | Covers |
|---|---|
| [Native Rust HDF engine](https://rascommander.info/ras/native-engine/) (ras-commander site) | User guide: engine modes, what is qualified, performance, limitations. |
| [Developer documentation](docs/README.md) | The engine itself: architecture, the Python module contract, building, testing and releasing. |
| [Changelog](CHANGELOG.md) | Changes in each release. |

## Quick start

Install ras-commander. On Windows x64, Linux x64 and macOS arm64 it installs
this engine as a dependency:

```bash
pip install ras-commander
```

With ras-commander 0.107.0, which does not install the engine automatically,
use `pip install ras-commander ras-commander-fast-hdf`.

Check that ras-commander has found it:

```python
from ras_commander import fast_hdf
print(fast_hdf.available())   # True: ras-commander now reads natively where it can
```

No code changes are needed. To choose the engine, set
`RAS_COMMANDER_FAST_HDF` (`h5py` turns the native engine off):

| Value | Behavior |
|---|---|
| `auto` (default) | Native where it is qualified; h5py whenever the engine refuses a read. |
| `h5py` | Never use the native engine. |
| `native` | Native or raise `FastHdfError`; for testing and qualification. |

## Status

| Item | Status |
|---|---|
| Version | **0.1.0**, Python extension `API_VERSION` 11. ras-commander accepts only the exact `API_VERSION` it was built against; ras-commander 0.107.0 and later use 11. |
| Platforms | CPython 3.10 or later (stable ABI). Windows x64 and Linux x64 are qualified; Linux wheels cover glibc 2.17 or later (`manylinux2014`) and musl (`musllinux_1_2`, Alpine). macOS arm64 builds in CI but has not been qualified against the HEC-RAS corpus. ras-commander 0.107.1 and later install the engine on these three platforms; elsewhere they use h5py. |
| Equivalence with h5py | Identical results to h5py on 201 public ras-commander method cases across 137 HEC-RAS 5.x–7.x result and geometry files: 3,800 method × file pairs in three modes, plus 1,428 calls replayed from the example notebooks. Methods the corpus has not exercised natively stay on h5py in `auto` mode. |
| Speed | 2D cell polygons 4–19× faster, depending on mesh size; mesh time series 2–7× faster on large meshes; small single reads a few milliseconds slower. See [Performance](#performance). |
| Not supported natively | Cross-section tables written before HEC-RAS 5.0.7, and names containing non-ASCII characters. ras-commander uses h5py for these in `auto` mode. |

## Performance

Timings were measured through ras-commander's public methods on a quiet
Windows workstation: 50 method × file cases, 600 fresh processes, h5py mode
against the default `auto` mode. Every case returned identical data in `h5py`,
`auto` and `native` mode.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/mesh-polygons-dark.svg">
  <img alt="Bar chart: HdfMesh.get_mesh_cell_polygons takes 17.2 s with h5py and 0.89 s natively on a 340,246-cell mesh, 4.34 s and 0.26 s on 87,039 cells, 0.28 s and 0.07 s on 4,425 cells, 0.23 s and 0.06 s on 3,314 cells." src="docs/images/mesh-polygons-light.svg" width="760">
</picture>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/speedup-by-method-dark.svg">
  <img alt="Dot chart of the native speed-up for 19 public methods: cell polygons 18.3×, mesh time series 5.1× to 7.5×, fluvial-pluvial boundary 1.5×, most bulk summary reads about 1.0×, and small reads 0.4× to 0.9×." src="docs/images/speedup-by-method-light.svg" width="760">
</picture>

Largest gains, repeat calls on the heaviest test file for each method:

| Method | Result rows | h5py | Native | Speed-up |
|---|---:|---:|---:|---:|
| `HdfMesh.get_mesh_cell_polygons` | 340,246 | 17,145 ms | 939 ms | 18.3× |
| `HdfResultsMesh.get_mesh_cells_timeseries` (water surface) | 1 | 1,372 ms | 184 ms | 7.5× |
| `HdfResultsMesh.get_mesh_timeseries` (water surface) | 193 | 1,415 ms | 213 ms | 6.6× |
| `HdfResultsMesh.get_mesh_timeseries` (face velocity) | 865 | 1,486 ms | 291 ms | 5.1× |
| `HdfFluvialPluvial.calculate_fluvial_pluvial_boundary` | 4,569 | 10,452 ms | 6,831 ms | 1.5× |

- **Where it helps:** building 2D cell polygons, and reading time series or
  sparse cells from large compressed meshes.
- **Where it does not help:** whole-array summaries such as `get_mesh_max_ws`
  run at about the same speed (within 6%), because both paths spend their time
  decompressing the same data.
- **Where it costs a little:** small reads (a few rows or one table) are up to
  16 ms slower on repeat calls, and up to about 0.13 s slower on the first call
  in a process, because each native read has a fixed setup cost. Across all
  50 cases the native engine is 1.07× faster on the first call in a process and
  1.30× faster on repeat calls (geometric mean).

Timings are with the files already in the operating system's cache. The full
table, method and environment are in [docs/performance.md](docs/performance.md).

## How it fits together

```text
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
instead of falling back, which is how qualification proves a workflow ran
natively. See [Architecture](docs/architecture.md).

## Repository layout

| Path | Contents |
|---|---|
| [`crates/ras-commander-rust`](crates/ras-commander-rust) | The engine: a Rust library with no Python dependency. It reads HEC-RAS project and plan files and the HDF5 files HEC-RAS writes (numeric, compound and string datasets, attributes, mesh topology, 1D and 2D results, structures, terrain modifications), and refuses anything it cannot read exactly. |
| [`python`](python) | The Python extension `ras_commander_fast_hdf` (PyO3 + maturin), built as the `ras-commander-fast-hdf` wheel. It exposes the engine to ras-commander, plus native mesh-polygon assembly, fluvial/pluvial kernels and PMTiles/vector exports. |
| [`vendor`](vendor) | A vendored, locally patched copy of the pure-Rust `hdf5-reader` and `hdf5-core` crates (MIT/Apache-2.0). See [`vendor/LOCAL_PATCHES.md`](vendor/LOCAL_PATCHES.md). |
| [`docs`](docs/README.md) | Developer documentation. |

## Building from source

Requirements: Rust 1.88 or later and Python 3.10 or later. On Windows, install
the Visual Studio C++ Build Tools.

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

## License

MIT; see [LICENSE](LICENSE). The wheel bundles the licenses of every linked
crate in [`python/THIRD-PARTY-NOTICES.md`](python/THIRD-PARTY-NOTICES.md).

ras-commander-fast-hdf is an independent open source project. HEC-RAS is
developed by the U.S. Army Corps of Engineers Hydrologic Engineering Center
(HEC). This project is not affiliated with, endorsed by, or supported by HEC
or USACE.
