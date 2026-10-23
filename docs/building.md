# Building

## Requirements

- Rust 1.88 or later (`rustup` stable is fine). The optional export writers in
  `python/src/exports` need Rust 1.95 or later.
- Python 3.10 or later and [maturin](https://www.maturin.rs/) 1.9 or later.
- Windows: Visual Studio C++ Build Tools. Linux and macOS: the system C
  toolchain. No HDF5 C library is needed; HDF5 decoding is pure Rust.

On a network share, point `CARGO_TARGET_DIR` at a local disk; builds on SMB
are slow and can fail on file locks.

## The wheel

```bash
pip install "maturin>=1.9,<2"
maturin build --release --locked \
    --manifest-path python/Cargo.toml \
    --features extension-module,exports-all,exports-pmtiles
pip install --force-reinstall target/wheels/ras_commander_fast_hdf-*.whl
python -c "import ras_commander_fast_hdf as n; print(n.__version__, n.API_VERSION, n.ENGINE_COMMIT)"
```

Release wheels are built with exactly these features. The wheel uses the
CPython 3.10 stable ABI (`abi3`), so one wheel per platform serves Python 3.10
and later.

| Feature | Adds |
|---|---|
| `extension-module` | Required for a Python wheel (maturin enables it). |
| `exports-pmtiles` | `export_pmtiles_ipc_file`. |
| `exports-all` | GeoJSON, FlatGeobuf, GeoParquet and GeoPackage writers (`export_ipc_file`, `validate_export_wkb`). |

For development, `maturin develop --release --manifest-path python/Cargo.toml
--features extension-module,exports-all,exports-pmtiles` builds and installs
into the active virtual environment in one step.

`ENGINE_COMMIT` comes from `git rev-parse HEAD` at build time. To build from a
source archive without Git metadata, set `RAS_ENGINE_COMMIT` to the full commit
hash; otherwise it is `"unknown"`.

### Linux wheels

Build a `manylinux_2_28` wheel, which runs on older distributions, with zig as
the linker (no Docker needed):

```bash
pip install ziglang
maturin build --release --locked --manifest-path python/Cargo.toml \
    --features extension-module,exports-all,exports-pmtiles \
    --zig --compatibility manylinux_2_28
```

## Continuous integration

Pushes and pull requests run on CLB Engineering's self-hosted runners (a
Slurm-scheduled pool for Linux and a disposable Windows VM). Pull requests
from forks never reach them; they run on GitHub-hosted runners instead. Each
run builds and tests the engine, the vendored reader and the extension, then
builds the Windows and Linux wheels and smoke-tests them on Python 3.10 and
3.13. macOS runs only for releases or when requested (`include_macos`).
Tagged releases build all three wheels (`.github/workflows/release.yml`).

## The engine alone

```bash
cargo build --release --all-features          # engine with every feature
cargo build --release --no-default-features --features catalog   # std-only catalog
```

## License notices

Two notice files must stay current; CI checks both.

```bash
# Engine (repository root)
python scripts/generate_third_party_notices.py --title "ras-commander-rust (all features)"

# Wheel
cd python
python scripts/generate_third_party_notices.py \
    --features extension-module,exports-all,exports-pmtiles \
    --own-licence ../LICENSE --title "ras-commander-fast-hdf wheel (release features)"
```

Run them after any `Cargo.lock` change. A crate whose package ships no license
file needs its upstream texts under `scripts/licence-fallbacks/<crate>-<version>/`
with a `SOURCE.txt`.
