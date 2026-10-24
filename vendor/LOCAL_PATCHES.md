# Local reader patches

`vendor/` holds a modified copy of the `hdf5-reader` and `hdf5-core` crates from
[roteiro-gis/netcdf-rust](https://github.com/roteiro-gis/netcdf-rust) at tag `v0.9.3`
(commit `3d708fd40c6809f5d215bd2d9f15e9a05de84bd0`, crate version 0.9.3), with the 14
patches in `patches/` applied in order. Copyright (c) 2026 Ian Norden. The upstream
licences (`LICENSE-MIT`, `LICENSE-APACHE`) and the upstream `README.md` are retained
byte for byte. None of these changes has been accepted upstream yet.

The change notice, listing every modified file and the reason for each change, is
[`hdf5-reader/PROVENANCE.md`](hdf5-reader/PROVENANCE.md). Per-file SHA-256 hashes of the
vendored tree are recorded in [`PROVENANCE.json`](PROVENANCE.json) and checked by
`scripts/prepare-reader.ps1` and `python scripts/vendor_sources.py --check`.

ras-commander-fast-hdf compiles the reader as private modules of its own crate. After the
patches, each Rust source file receives a two-line change notice and module-rooted paths
(`crate::hdf5_reader::`, `crate::hdf5_core::`), and the reader's `rayon`/`lz4` feature
tests become the crate's `hdf` feature; see "Embedding" in `hdf5-reader/PROVENANCE.md`.
`vendor/` also holds `freestiler-core` (see `freestiler-core/PROVENANCE.md`).

## Reproducing the tree

```bash
git clone https://github.com/roteiro-gis/netcdf-rust
git clone https://github.com/walkerke/freestiler
python scripts/vendor_sources.py --netcdf-rust netcdf-rust --freestiler freestiler          # compare
python scripts/vendor_sources.py --netcdf-rust netcdf-rust --freestiler freestiler --write  # replace
```

The script checks out `3d708fd` in a temporary worktree, applies `patches/*.patch` with
`git am`, applies the embedding rules, compares the result with `vendor/` file by file and
rewrites `PROVENANCE.json` (it does the same for freestiler at `beb64b0`, without patches).

## Testing the reader on its own

```bash
cargo test --manifest-path vendor/reader-tests/Cargo.toml
```

runs the reader's in-source unit tests and upstream's integration, corruption and property
tests (`hdf5-reader/tests/`, fixtures in `testdata/hdf5/`) against the embedded source.

## Patch series

Each patch starts with a header that gives its status against upstream v0.9.3, its kind
and a proposed upstream grouping. Patch 0006 also adjusts four `OpenOptions` literals in
upstream's `netcdf-reader`; that crate is not vendored, so those hunks have no effect here.

| Patch | Change | Kind |
|---|---|---|
| `0001` | Read the location of v2/v3 shared messages directly after the type byte, and accept type 2 (committed) for v2 messages. Without it, any HDF5 1.8+ dataset with a committed datatype fails to open. | spec fix |
| `0002` | Keep exact datatype envelopes (`DatatypeMessage::raw_encoding`, `AttributeMessage::raw_datatype`); add `Group::raw_attributes`, `Dataset::raw_attributes`, `Dataset::raw_datatype` and `Error::UnsupportedEncoding`; bound attribute parsing to the message size and reject reserved attribute flags. | API + hardening |
| `0003` | Check an attribute's declared element count against its stored payload before allocating; refuse zero-width fixed strings. | safety |
| `0004` | Clear an in-flight chunk-cache entry from a drop guard, so a panicking decoder cannot block later loads of the chunk. | safety |
| `0005` | Checked arithmetic for file-derived sizes in fixed and extensible array indexes, the global heap, object-header continuations and the N-Bit and scale-offset filters. | safety |
| `0006` | `OpenOptions::max_chunk_bytes` (256 MiB default, clamped to 4 GiB) checked before any chunk allocation; `Error::ResourceLimit`; every built-in filter call clamped to the format maximum. | safety + API |
| `0007` | Bound each stage of a multi-filter pipeline by the encoded-size bound of the earlier filters (HEC-RAS writes DEFLATE twice on small chunks). Adds `tests/fixtures/scaleoffset_deflate.h5`. | spec fix |
| `0008` | **Engine policy.** Refuse custom, unknown and LZ4-prefix filters whenever a decoded-size limit applies. | policy |
| `0009` | Ignore well-formed, grid-aligned stale chunk-index entries wholly outside a shrunk dataset's extent. | fix |
| `0010` | One link walk per soft/external link resolution: shared depth limit, cycle guard and expansion budget (upstream overflows the stack on `/s -> /s/x`). | safety |
| `0011` | Cache old-style (symbol-table) group names in the file context; read the local heap once; add `Group::member_names`. Adds `testdata/empty_heap_past_eof.h5`. | performance + API |
| `0012` | Follow soft links in old-style groups (symbol-table cache type 2). | spec fix |
| `0013` | Sparse row/column gathers (`read_rows_columns_chunked*`), uncached full reads (`read_array_uncached*`), zero-byte cache bypass, `DEFAULT_CHUNK_CACHE_{BYTES,SLOTS}`. Adds `tests/fixtures/perf2/gather.h5`. | performance + API |
| `0014` | **Engine policy.** Refuse extensible-array chunk indexes with more elements than the index block (`UnsupportedEncoding`), because upstream 0.9.3 silently returns fill values for chunks past the index block. Adjusts two upstream integration tests. Adds `tests/fixtures/perf2/ea_latest.h5`. | policy |

Patches 0008 and 0014 are fail-closed qualification policies of the engine and are not
intended for upstream as they stand.

## Changes from the previous vendored copy

The previous copy was upstream `24ff6fc` (0.9.1 plus upstream PR #95) with 15 local
changes. Moving to v0.9.3:

- brings upstream PRs #90, #91 (fractal-heap huge objects and short indirect heap IDs),
  #92 (creation-order member listing), #93 (vlen strings with a one-byte base), #94
  (filtered chunk sizes as 1-8 byte integers in fixed arrays, extensible arrays and v2
  B-trees) and #100 (fallible selection buffers; filters fail instead of aborting);
- drops the local fixed-array chunk-size-width patch (superseded by #94) and the local
  DEFLATE/LZ4 allocation changes (superseded by #100);
- drops test-only CLB additions (`cfg(test)` work counters, the `perf2b_work_counters`
  test and `tests/malformed.rs`; the engine's `tests/malformed_inputs.rs` covers the same
  files);
- keeps upstream's own integration, corruption and property tests (`hdf5-reader/tests/`)
  and the fixtures they read (`testdata/hdf5/`, from upstream `testdata/hdf5`).

## Rules

Change the vendored source only by adding a patch to `patches/`, regenerating with
`scripts/vendor_sources.py` and describing the change in `hdf5-reader/PROVENANCE.md` and
this file. Never edit files under `vendor/*/src` by hand.
