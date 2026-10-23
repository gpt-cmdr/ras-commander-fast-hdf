# Source provenance

This page explains where the code in this repository came from and which licence notices
apply to it. In short: the Rust engine was extracted from the MIT-licensed
[ras-commander](https://github.com/gpt-cmdr/ras-commander) project, the Python extension
came from ras-commander's `rust/fast-hdf` directory, and the pure-Rust HDF5 reader is a
vendored, locally modified copy of an upstream MIT/Apache-2.0 project.

## Engine (`crates/ras-commander-rust`)

The engine began as a native extraction of HDF, catalogue and mesh logic from
[gpt-cmdr/ras-commander](https://github.com/gpt-cmdr/ras-commander) (MIT). It was then
developed privately by CLB Engineering Corporation until this public release. Porting
preserved the MIT copyright notices and attribution of the extracted code. This repository
does not claim authorship of upstream storage libraries or of algorithms inherited from
ras-commander.

## Python extension (`python/`)

The Python extension (`ras_commander._fast_hdf`) was previously maintained in
ras-commander's `rust/fast-hdf` directory and moved here together with the engine. It now
depends on the engine through a Cargo path dependency on `../crates/ras-commander-rust`.

## Vendored HDF5 reader (`vendor/`)

`vendor/` contains a modified copy of the `hdf5-reader` and `hdf5-core` crates from
[roteiro-gis/netcdf-rust](https://github.com/roteiro-gis/netcdf-rust) (`MIT OR
Apache-2.0`, copyright Ian Norden), based on upstream commit
`24ff6fcfcc79a35c30aa5d8cb3028ab4d9d00564`. The base requires the local shared-message and
filtered fixed-array width fixes to read the qualified HEC-RAS corpus. Generic reader
changes are kept generic: no RAS-specific parsing is hidden in the reader.

- [`vendor/hdf5-reader/PROVENANCE.md`](../../vendor/hdf5-reader/PROVENANCE.md) is the
  change notice for every modified file.
- [`vendor/LOCAL_PATCHES.md`](../../vendor/LOCAL_PATCHES.md) summarises the patch series.
- [`vendor/PROVENANCE.json`](../../vendor/PROVENANCE.json) records the upstream, base
  commit and per-file SHA-256 hashes, which `scripts/prepare-reader.ps1` verifies.

## Copyright and licence notices

- [`LICENSE`](../../LICENSE): MIT, copyright CLB Engineering Corporation. This covers the
  repository as a whole.
- [`python/licenses/ras-commander-LICENSE.txt`](../../python/licenses/ras-commander-LICENSE.txt):
  MIT, copyright William M. Katzenmeyer. This is the original ras-commander notice and is
  retained for the code extracted from ras-commander.
- `vendor/LICENSE-MIT` and `vendor/LICENSE-APACHE`: the upstream licences of the vendored
  reader, retained unchanged. The reader is redistributed under the MIT option.
- `THIRD-PARTY-NOTICES.md` (repository root) and `python/THIRD-PARTY-NOTICES.md` are the
  generated dependency licence bundles for the engine and for the wheel.

Freestiler, used by the Python extension for vector tiles, is an external dependency; its
own limitations are not changed here.

## What is not committed

No extracted HDF model datasets, compiled binaries, credentials, Python environments or
benchmark outputs belong in the repository. Test fixtures are small synthetic files or
small excerpts of public HEC-RAS example projects and public FEMA/NOAA models, each with
recorded hashes and provenance; unavailable and unsupported cases are explicit.
