# Changelog

## 0.2.0 (unreleased, pending qualification)

- One Rust crate, `ras-commander-fast-hdf` (Rust import
  `ras_commander_fast_hdf`), at the repository root, ready to publish on
  crates.io. It replaces the engine crate `ras-commander-rust`, the separate
  extension crate in `python/`, the export crate `ras-export` and the vendored
  `hdf5-reader`/`hdf5-core` path crates. The name `ras-commander-rust` is
  retired. Rust users enable `hdf`, `mesh` and `results` as needed (default:
  `catalog`, standard library only); without the `python` feature no PyO3 or
  NumPy is in the dependency tree.
- The PyPI wheel is built from the same crate (`python/pyproject.toml`, features
  `python,extension-module`; release wheels add `exports-all,exports-pmtiles`).
  The Python API and behaviour are unchanged from 0.1.1; extension
  `API_VERSION` stays 11 and engine `API_VERSION` stays 10.
- HDF5 reader rebased from `hdf5-reader` 0.9.1 and its older local patch set
  onto upstream 0.9.3 (roteiro-gis/netcdf-rust `v0.9.3`) plus 14 local patches
  (`vendor/patches/`, described in `vendor/LOCAL_PATCHES.md`).
- `freestiler-core` (walkerke/freestiler `beb64b0`), previously a Git
  dependency, is embedded from `vendor/freestiler-core`. No Git dependencies
  remain.
- Embedded third-party source is compiled as private modules, recorded with
  hashes in `vendor/PROVENANCE.json` and checked by
  `scripts/vendor_sources.py --check`. Upstream's reader tests run from
  `vendor/reader-tests`.
- Layout: engine in `src/`, Python bridge in `src/python/` (was `python/src`),
  export writers in `src/exports/`, all tests in `tests/` with fixtures in
  `tests/fixtures`, `build.rs` at the root, notice scripts in `scripts/`.
- Licence notices: `THIRD-PARTY-NOTICES.md` (crate, all features) and
  `python/THIRD-PARTY-NOTICES.md` (wheel) come from one script,
  `scripts/generate_third_party_notices.py`. The engine is still listed there
  under its former name `ras-commander-rust`, which ras-commander's wheel
  check looks for.

## 0.1.1 (2026-10-24)

- Linux wheels for older and musl-based distributions: `manylinux2014`
  (glibc 2.17 or later) replaces `manylinux_2_28`, and a new `musllinux_1_2`
  wheel covers Alpine. `pip install ras-commander` no longer tries to compile
  the engine on those systems. Engine and extension code are unchanged.

- Documentation: this package is ras-commander's default HDF engine, not an
  optional extension. ras-commander 0.107.1 and later list it as a core
  dependency on Windows x64, Linux x64 and macOS arm64 (CPython), so
  `pip install ras-commander` installs it; on other platforms ras-commander
  uses h5py. `RAS_COMMANDER_FAST_HDF=h5py` still turns it off. The package
  itself is unchanged.

## 0.1.0 (2026-10-23)

First public release. The engine (previously the private
`ras-commander-rust` repository) and the Python extension (previously
`rust/fast-hdf` in ras-commander) now live together in this repository.

- Extension `API_VERSION` 11, engine `API_VERSION` 10. Works with
  ras-commander 0.107.0 and later.
- Native reads for ras-commander's plan results, geometry, terrain and
  land-cover HDF access: numeric, compound and fixed-length string datasets,
  attributes, 2D mesh geometry, summaries, time series and boundary
  conditions, cross sections and 1D results, structures, reference lines and
  points, terrain modifications and stored classification tables.
- Native 2D cell polygon assembly, fluvial/pluvial kernels, PMTiles and vector
  exports (GeoJSON, FlatGeobuf, GeoParquet, GeoPackage), and RAS text-file
  parsing.
- The wheel's `THIRD-PARTY-NOTICES.md` covers every bundled crate. `epsg-utils`
  (used by the GeoPackage writer) declares MIT but publishes no licence text;
  its notice reproduces the MIT text with an explanation.
- Qualified identical to h5py through ras-commander on 137 HEC-RAS 5.x–7.x
  files (3,800 method × file pairs in three modes) and 1,428 notebook calls,
  on Windows and Linux.
- `ENGINE_COMMIT` is now the commit of this repository the wheel was built
  from. Wheels built before the move report the private engine commit
  `0b024adc7857e9fb2d364997ad002e01a939cec1`, whose engine source is identical.
