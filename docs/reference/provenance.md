# Source provenance

This page explains where the code in this repository came from and which licence notices
apply to it. In short: the Rust engine was extracted from the MIT-licensed
[ras-commander](https://github.com/gpt-cmdr/ras-commander) project, the Python extension
came from ras-commander's `rust/fast-hdf` directory, and the pure-Rust HDF5 reader is a
vendored, locally modified copy of an upstream MIT/Apache-2.0 project. Since 0.2.0 all of it
is one crate, `ras-commander-fast-hdf`.

## Engine (`src/`)

The engine began as a native extraction of HDF, catalogue and mesh logic from
[gpt-cmdr/ras-commander](https://github.com/gpt-cmdr/ras-commander) (MIT). It was then
developed privately by CLB Engineering Corporation until this public release. Porting
preserved the MIT copyright notices and attribution of the extracted code. This repository
does not claim authorship of upstream storage libraries or of algorithms inherited from
ras-commander. Until 0.2.0 it was the separate crate `ras-commander-rust`; that name is
retired.

## Python extension (`src/python/`)

The Python extension (`ras_commander._fast_hdf`) was previously maintained in
ras-commander's `rust/fast-hdf` directory and moved here together with the engine. It is
now the `python` feature of the same crate; `python/` holds only the Python packaging.

## Vendored source (`vendor/`)

The vendored source is compiled into the crate as private modules (`src/lib.rs`); none of
it is public API. `vendor/` contains a modified copy of the `hdf5-reader` and `hdf5-core`
crates from
[roteiro-gis/netcdf-rust](https://github.com/roteiro-gis/netcdf-rust) (`MIT OR
Apache-2.0`, copyright Ian Norden), based on upstream tag `v0.9.3` (commit
`3d708fd40c6809f5d215bd2d9f15e9a05de84bd0`) with 14 local patches (`vendor/patches/`).
Generic reader changes are kept generic: no RAS-specific parsing is hidden in the reader.
Two patches are fail-closed engine policies (unbounded filters and non-inline
extensible-array indexes are refused).

- [`vendor/hdf5-reader/PROVENANCE.md`](https://github.com/gpt-cmdr/ras-commander-fast-hdf/blob/main/vendor/hdf5-reader/PROVENANCE.md) is the
  change notice for every modified file.
- [`vendor/LOCAL_PATCHES.md`](https://github.com/gpt-cmdr/ras-commander-fast-hdf/blob/main/vendor/LOCAL_PATCHES.md) summarises the patch series.
- [`vendor/PROVENANCE.json`](https://github.com/gpt-cmdr/ras-commander-fast-hdf/blob/main/vendor/PROVENANCE.json) records the upstream, base
  commit and per-file SHA-256 hashes. `scripts/vendor_sources.py --check` (or
  `scripts/prepare-reader.ps1`) verifies `vendor/` against it.

`vendor/freestiler-core` is a mechanically adapted copy of `freestiler-core` from
[walkerke/freestiler](https://github.com/walkerke/freestiler) at revision
`beb64b0d4414da895ec67d02b2f91db6ab6acc37` (MIT AND ISC), used by the Python extension for
vector tiles. Its crate is not published, so it is embedded instead of being a Git
dependency; `vendor/freestiler-core/PROVENANCE.md` lists the changes.

## Copyright and licence notices

- [`LICENSE`](https://github.com/gpt-cmdr/ras-commander-fast-hdf/blob/main/LICENSE): MIT, copyright CLB Engineering Corporation. This covers the
  repository as a whole.
- [`python/licenses/ras-commander-LICENSE.txt`](https://github.com/gpt-cmdr/ras-commander-fast-hdf/blob/main/python/licenses/ras-commander-LICENSE.txt):
  MIT, copyright William M. Katzenmeyer. This is the original ras-commander notice and is
  retained for the code extracted from ras-commander.
- `vendor/LICENSE-MIT` and `vendor/LICENSE-APACHE`: the upstream licences of the vendored
  reader, retained unchanged. The crate's licence expression keeps the reader's
  `MIT OR Apache-2.0`.
- `vendor/freestiler-core/LICENSE`, `LICENSE.md`, `LICENSE.note` and `COPYRIGHTS`: the
  freestiler licence texts, retained unchanged.
- `THIRD-PARTY-NOTICES.md` (repository root) and `python/THIRD-PARTY-NOTICES.md` are the
  generated dependency licence bundles for the crate (all features) and for the wheel.

## What is not committed

No extracted HDF model datasets, compiled binaries, credentials, Python environments or
benchmark outputs belong in the repository. Test fixtures are small synthetic files or
small excerpts of public HEC-RAS example projects and public FEMA/NOAA models, each with
recorded hashes and provenance; unavailable and unsupported cases are explicit.
