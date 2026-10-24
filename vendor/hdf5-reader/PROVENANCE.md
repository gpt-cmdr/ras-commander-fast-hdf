# hdf5-reader / hdf5-core: provenance and change notice

`vendor/hdf5-reader` and `vendor/hdf5-core` hold a modified copy of the `hdf5-reader` and
`hdf5-core` crates from **roteiro-gis/netcdf-rust**, copyright (c) 2026 Ian Norden,
licensed `MIT OR Apache-2.0`. Both licence texts are retained unchanged in
`vendor/LICENSE-MIT` and `vendor/LICENSE-APACHE`. Upstream has no `NOTICE` file.

This file is the change notice required by Apache-2.0 section 4(b): it states that the
files listed below were changed, by whom and why. The changes are by CLB Engineering
Corporation for ras-commander-fast-hdf and are offered under the same
`MIT OR Apache-2.0` terms as the upstream code.

## Upstream

| | |
|---|---|
| Upstream repository | https://github.com/roteiro-gis/netcdf-rust |
| Base | tag `v0.9.3`, commit `3d708fd40c6809f5d215bd2d9f15e9a05de84bd0` ("Merge pull request #101 ... release-0.9.3"; crate version 0.9.3) |
| Local patches | `vendor/patches/0001`-`0014`, applied in order with `git am` |
| Hashes | `vendor/PROVENANCE.json` (every vendored file); verify with `scripts/prepare-reader.ps1` or `python scripts/vendor_sources.py --check` |

## Modified files

Files changed by the patch series, with the patches that change them:

| File | Patches |
|---|---|
| `hdf5-core/src/lib.rs` | 0002 |
| `hdf5-reader/src/attribute_api.rs` | 0003 |
| `hdf5-reader/src/cache.rs` | 0004, 0013 |
| `hdf5-reader/src/dataset.rs` | 0002, 0006, 0009, 0011, 0013 |
| `hdf5-reader/src/error.rs` | 0002, 0006 |
| `hdf5-reader/src/extensible_array.rs` | 0005, 0014 |
| `hdf5-reader/src/filters/mod.rs` | 0006, 0007, 0008 |
| `hdf5-reader/src/filters/nbit.rs` | 0005 |
| `hdf5-reader/src/filters/scaleoffset.rs` | 0005 |
| `hdf5-reader/src/fixed_array.rs` | 0005 |
| `hdf5-reader/src/global_heap.rs` | 0005 |
| `hdf5-reader/src/group.rs` | 0002, 0010, 0011, 0012 |
| `hdf5-reader/src/lib.rs` | 0006, 0011, 0013 |
| `hdf5-reader/src/local_heap.rs` | 0011 |
| `hdf5-reader/src/messages/attribute.rs` | 0002 |
| `hdf5-reader/src/messages/datatype.rs` | 0002 |
| `hdf5-reader/src/messages/shared.rs` | 0001 |
| `hdf5-reader/src/object_header.rs` | 0005 |
| `hdf5-reader/tests/integration.rs` | 0014 (two extensible-array cases now expect the policy refusal) |

Added by the patches: `hdf5-reader/testdata/empty_heap_past_eof.h5` (0011),
`hdf5-reader/tests/fixtures/scaleoffset_deflate.h5` and `.json` (0007),
`hdf5-reader/tests/fixtures/perf2/gather.h5` (0013) and
`hdf5-reader/tests/fixtures/perf2/ea_latest.h5` (0014). All are small synthetic files.

No other file is changed by the patches. Files under `hdf5-reader/tests`,
`hdf5-reader/testdata` and `testdata/hdf5` are byte-for-byte upstream (patched) copies;
every `.rs` file under `hdf5-core/src` and `hdf5-reader/src` also carries the embedding
changes described next.

## Embedding in ras-commander-fast-hdf

The crate compiles `hdf5-core/src` and `hdf5-reader/src` as its private modules
`crate::hdf5_core` and `crate::hdf5_reader` (see `src/lib.rs`), so the published crate
has no dependency on a separate reader crate. To make that possible every `.rs` file
under those two directories was changed mechanically, after the patches, by
`scripts/vendor_sources.py`:

1. A two-line notice naming the upstream, its licence and copyright, and this file is
   prepended (the "prominent notice" of Apache-2.0 section 4(b)).
2. Crate-relative paths are re-rooted at the module: `crate::` becomes
   `crate::hdf5_reader::` (or `crate::hdf5_core::` in hdf5-core), and the reader's
   `hdf5_core::` paths become `crate::hdf5_core::`.
3. The reader's optional features `rayon` and `lz4`, both enabled by upstream's default
   features and by the engine, are tested as the crate's `hdf` feature:
   `feature = "rayon"` and `feature = "lz4"` become `feature = "hdf"`.

Nothing else differs. `vendor/PROVENANCE.json` records, for each embedded file, the
SHA-256 of the patched upstream file and of the embedded result, and the exact rules.
Upstream's tests (`hdf5-reader/tests/`) and fixtures are copied unchanged; the harness
`vendor/reader-tests` compiles the embedded modules the same way and runs those tests
against them.

The crate keeps upstream's safety lints for this module: `unsafe_op_in_unsafe_fn` and
`clippy::undocumented_unsafe_blocks` are denied. It is the only module of the crate
allowed to use `unsafe` (memory-mapped reads and copies of decoded chunks into
preallocated buffers).

Written locally (not upstream files): `vendor/reader-tests/` (Cargo.toml, Cargo.lock,
src/lib.rs), this file, `vendor/LOCAL_PATCHES.md` and `vendor/PROVENANCE.json`.

## Why each change was made

See `vendor/LOCAL_PATCHES.md` for the table, and the header of each patch file for its
full description, its status against upstream v0.9.3 and a proposed upstream grouping.
In short: 0001, 0007, 0009 and 0012 fix reads of valid HDF5 files; 0003, 0004, 0005,
0006 and 0010 harden the reader against malformed files; 0002, 0011 and 0013 add the
APIs the engine uses (raw encodings, cheap group listings, sparse gathers); 0008 and
0014 are the engine's fail-closed qualification policies.
