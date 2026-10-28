# Source provenance

This page explains where the code in this repository came from and which licence notices
apply to it. In short: the Rust engine was extracted from the MIT-licensed
[ras-commander](https://github.com/gpt-cmdr/ras-commander) project, the Python extension
came from ras-commander's `rust/fast-hdf` directory, and the pure-Rust HDF5 reader is the
upstream MIT/Apache-2.0 `hdf5-reader` crate, an ordinary crates.io dependency. Since 0.2.0
the engine and the extension are one crate, `ras-commander-fast-hdf`.

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

## HDF5 reader

HDF5 decoding uses the `hdf5-reader` and `hdf5-core` crates 0.10.1 from
[roteiro-gis/netcdf-rust](https://github.com/roteiro-gis/netcdf-rust) (`MIT OR
Apache-2.0`, copyright Ian Norden; tag `v0.10.1`, commit
`957b10000b1bb52654a51881b3fb868a32f26b7e`), as ordinary crates.io dependencies with the
reader's `rayon` and `lz4` features. Up to 0.2.3 the engine compiled a copy of the 0.9.3
reader with 14 local patches into the crate. Twelve of them are upstream in 0.10.0 and
0.10.1:

| Old patch | Upstream 0.10.0 / 0.10.1 |
|---|---|
| 0001 v2/v3 shared message locations | #103 (`526a88b`) |
| 0002 exact datatype bytes, raw attributes, `UnsupportedEncoding`, attribute bounds | #108 (`e6d374d`), #109 (`d343783`) |
| 0003 attribute element counts | #104 (`ec2e4dc`) |
| 0004 in-flight chunk loads after a decoder panic | #104 (`a2a2a63`) |
| 0005 checked arithmetic for file-derived sizes | #104 (`4e3705c`) |
| 0006 per-chunk allocation limit, `ResourceLimit` | #104 (`7698a51`, `52011e7`) |
| 0007 multi-filter stage bounds | #103 (`d0a51fa`) |
| 0009 stale chunks outside a shrunk extent | #103 (`ac1572e`) |
| 0010 one link walk per soft/external link | #104 (`9639349`) |
| 0011 old-style group name cache, `Group::member_names` | #107 (`5242e51`) |
| 0012 soft links in old-style groups | #103 (`8c1c6d8`) |
| 0013 row/column gathers, uncached reads, cache defaults | #106 (`1be2681`); selected-chunk index lookups for gathers: #111 (`269fcae`, 0.10.1) |

The other two were CLB qualification policies, not fixes:

- **Unbounded filters** (old 0008), applied by the engine. In a pipeline of several filters, a filter without an
  encoded-size bound (LZ4, SZIP or an unknown filter) that is decoded as an intermediate
  stage would be decoded without a size limit. The read is refused (`Unsupported`, code
  `encoding`).
- **Large extensible-array chunk indexes** (old 0014). The 0.9.3 reader returned fill
  values for chunks past the index block, so such indexes were refused. Upstream 0.10.0
  maps them correctly (#102, #105), so they are read; every value of the committed
  extensible-array fixtures is checked against h5py.

The unbounded-filter policy is applied by the engine itself, with the same refusals. All
reads go through `src/hdf/reader.rs`, which checks it before the reader
reads a dataset's chunk index or decodes a chunk. Metadata reads are not affected. The
policy is judged per dataset rather than per chunk, so it is slightly stricter than
before (for example, a pipeline whose chunks all skip the unbounded filter is refused too).
No HEC-RAS file is affected: HEC-RAS writes v1 B-tree chunk indexes and no LZ4 or unknown
filters. The tests are `src/hdf/reader_policy_tests.rs`, with fixtures in
`tests/fixtures/reader_policy`.

## Vendored source (`vendor/`)

The vendored source is compiled into the crate as a private module (`src/lib.rs`); none of
it is public API. [`vendor/PROVENANCE.json`](https://github.com/gpt-cmdr/ras-commander-fast-hdf/blob/main/vendor/PROVENANCE.json)
records the upstream, base commit and per-file SHA-256 hashes.
`scripts/vendor_sources.py --check` (or `scripts/prepare-reader.ps1`) verifies `vendor/`
against it.

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
- `vendor/freestiler-core/LICENSE`, `LICENSE.md`, `LICENSE.note` and `COPYRIGHTS`: the
  freestiler licence texts, retained unchanged.
- `THIRD-PARTY-NOTICES.md` (repository root) and `python/licenses/THIRD-PARTY-NOTICES.md` are the
  generated dependency licence bundles for the crate (all features) and for the wheel.

## What is not committed

No extracted HDF model datasets, compiled binaries, credentials, Python environments or
benchmark outputs belong in the repository. Test fixtures are small synthetic files or
small excerpts of public HEC-RAS example projects and public FEMA/NOAA models, each with
recorded hashes and provenance; unavailable and unsupported cases are explicit.
