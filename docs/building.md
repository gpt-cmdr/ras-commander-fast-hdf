# Building

## Requirements

- Rust 1.88 or later (`rustup` stable is fine). The optional export writers
  (`exports-*` features, `src/exports`) need Rust 1.95 or later.
- Python 3.10 or later and [maturin](https://www.maturin.rs/) 1.9 or later.
- Windows: Visual Studio C++ Build Tools. Linux and macOS: the system C
  toolchain. No HDF5 C library is needed; HDF5 decoding is pure Rust.

On a network share, point `CARGO_TARGET_DIR` at a local disk; builds on SMB
are slow and can fail on file locks.

## One crate, two packages

The repository is one Rust crate, `ras-commander-fast-hdf`. Cargo builds it as
a Rust library; maturin builds the same crate as the Python wheel, through
`python/pyproject.toml` (`manifest-path = "../Cargo.toml"`, features
`python,extension-module`).

| Feature | Adds |
|---|---|
| `catalog` (default) | Project and plan text catalogue; standard library only. |
| `hdf` | HDF5 reads; pure Rust, with the `hdf5-reader` crate. |
| `mesh` | 2D mesh geometry and topology (implies `hdf`). |
| `results` | 1D/2D results, summaries and time series (implies `hdf`). |
| `python` | PyO3 bindings, the kernels only ras-commander uses and the embedded `freestiler-core`; implies the four engine features. |
| `extension-module` | `python` plus `pyo3/extension-module`; required for a wheel (maturin enables it). |
| `exports-arrow` | Arrow IPC input for the export writers (implies `python`). |
| `exports-geojson`, `exports-flatgeobuf`, `exports-geoparquet`, `exports-geopackage` | One vector writer each (`export_ipc_file`, `validate_export_wkb`). |
| `exports-pmtiles` | `export_pmtiles_ipc_file`. |
| `exports-all` | The four writers above (not PMTiles). |
| `arrow` | Canonical HEC-RAS layers as Arrow record batches (`arrow_layers`); implies `hdf`, `mesh`, `results`. No PyO3. |
| `python-arrow` | `arrow_layer` and `layer_catalog`: the layers as Arrow C streams (PyCapsule) in Python; implies `python` and `arrow`. Release wheels add it with the exports features. |
| `test-hooks` | Panic-injection hooks for tests (`_panic_for_test` on the module and on `HdfSession`; implies `python`). CI test builds enable it; release wheels and crates do not. |

Without `python`, no PyO3 or NumPy is in the dependency tree.

## The wheel

```bash
pip install "maturin>=1.9,<2"
cd python
maturin build --release --locked \
    --features extension-module,exports-all,exports-pmtiles --out ../dist
pip install --force-reinstall ../dist/ras_commander_fast_hdf-*.whl
python -c "import ras_commander_fast_hdf as n; print(n.__version__, n.API_VERSION, n.ENGINE_COMMIT)"
```

Release wheels are built with exactly these features. The wheel uses the
CPython 3.10 stable ABI (`abi3`), so one wheel per platform serves Python 3.10
and later. Build the source distribution with
`cd python && maturin sdist --out ../dist`.

For development, `maturin develop --release --features
extension-module,exports-all,exports-pmtiles`, run in `python/`, builds and
installs into the active virtual environment in one step. Add `test-hooks` to
the features to run ras-commander's panic-containment tests
(`tests/rust_engine_bridge/panic_guard_test.py`); they skip on a release wheel.

`ENGINE_COMMIT` is the commit of this checkout, read by `build.rs` at build
time. To build from a
source archive without Git metadata, set `RAS_ENGINE_COMMIT` to the full commit
hash; otherwise it is `"unknown"`.

### Linux wheels

Release wheels use zig as the linker (no Docker needed): a `manylinux2014`
wheel, which runs on any distribution with glibc 2.17 or later, and a
`musllinux_1_2` wheel for Alpine and other musl distributions.

```bash
pip install ziglang
cd python
maturin build --release --locked --out ../dist \
    --features extension-module,exports-all,exports-pmtiles \
    --zig --compatibility manylinux2014

rustup target add x86_64-unknown-linux-musl
maturin build --release --locked --out ../dist-musl \
    --features extension-module,exports-all,exports-pmtiles \
    --zig --target x86_64-unknown-linux-musl --compatibility musllinux_1_2
```

CI installs both in containers (an old-glibc manylinux2014 image and Alpine)
and smoke-tests them.

## Continuous integration

Pushes and pull requests run on CLB Engineering's self-hosted runners (a
Slurm-scheduled pool for Linux and a disposable Windows VM). Pull requests
from forks never reach them; they run on GitHub-hosted runners instead. Each
run tests the pure-Rust engine profiles, the extension, the vendored source
(`scripts/vendor_sources.py --check`) and the licence
notices, packages the crate (`cargo publish --locked --dry-run`), then builds
the Windows and Linux wheels and smoke-tests them on Python 3.10 and
3.13. macOS runs only for releases or when requested (`include_macos`).
Tagged releases build all three wheels (`.github/workflows/release.yml`).

## The crate alone

```bash
cargo build --release                                     # default: catalog only
cargo build --release --features hdf,mesh,results         # whole engine, pure Rust
cargo publish --locked --dry-run                          # crates.io package check
```

Rust users add it with `cargo add ras-commander-fast-hdf`, or in `Cargo.toml`:

```toml
ras-commander-fast-hdf = { version = "0.2", features = ["hdf", "mesh", "results"] }
```

## Licence notices

Two notice files must stay current; CI checks both (the same commands with
`--check`).

```bash
# Crate, all features: THIRD-PARTY-NOTICES.md
python scripts/generate_third_party_notices.py --title "ras-commander-fast-hdf (all features)"

# Wheel, release features: python/licenses/THIRD-PARTY-NOTICES.md
python scripts/generate_third_party_notices.py \
    --features python,extension-module,exports-all,exports-pmtiles \
    --title "ras-commander-fast-hdf wheel (release features)" \
    --output python/licenses/THIRD-PARTY-NOTICES.md
```

Run them from the repository root after any `Cargo.lock` change. Both list the
engine as `ras-commander-fast-hdf`, the name ras-commander's installed-wheel
check looks for. A crate whose package ships no license
file needs its upstream texts under `scripts/licence-fallbacks/<crate>-<version>/`
with a `SOURCE.txt`.
