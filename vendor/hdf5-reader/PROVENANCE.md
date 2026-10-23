# hdf5-reader / hdf5-core: provenance and local changes

This directory holds a modified copy of the `hdf5-reader` and `hdf5-core` crates from
**roteiro-gis/netcdf-rust**. The original copyright (Ian Norden) and the original
`MIT OR Apache-2.0` licence files (`vendor/LICENSE-MIT`, `vendor/LICENSE-APACHE`) are
retained unchanged. This file is the change notice for every modified file: it lists what
differs from upstream and why.

## Upstream

| | |
|---|---|
| Upstream repository | https://github.com/roteiro-gis/netcdf-rust (named in `hdf5-reader/Cargo.toml`) |
| Vendored base | `24ff6fcfcc79a35c30aa5d8cb3028ab4d9d00564` "Read v1/v2 compound member names and old fill value messages per the spec" (crate version 0.9.1) |
| Upstream head compared | `roteiro-gis/netcdf-rust` `main` `3d708fd` (v0.9.3, 2026-10-20) |

`vendor/PROVENANCE.json` records the upstream, the base commit and the SHA-256 of every
file in `vendor/` (except itself); `scripts/prepare-reader.ps1` verifies those hashes.
`vendor/LOCAL_PATCHES.md` summarises the local patch series.

## Files that differ from upstream

Compared with line endings normalised. Of the 49 source files, 29 are identical to the
base and 20 differ:

- `hdf5-core/src/lib.rs`
- `attribute_api.rs`
- `cache.rs`
- `dataset.rs`
- `error.rs`
- `extensible_array.rs`
- `filters/deflate.rs`
- `filters/lz4.rs`
- `filters/mod.rs`
- `filters/nbit.rs`
- `filters/scaleoffset.rs`
- `fixed_array.rs`
- `global_heap.rs`
- `group.rs`
- `lib.rs`
- `local_heap.rs`
- `messages/attribute.rs`
- `messages/datatype.rs`
- `messages/shared.rs`
- `object_header.rs`

Against upstream `main` (v0.9.3), `btree_v2.rs` and `fractal_heap.rs` also differ. Those are
upstream changes made after the base that have not been taken (listed below).

`hdf5-reader/Cargo.toml` is adapted from workspace inheritance to a standalone manifest.
`hdf5-reader/tests/` and `hdf5-reader/testdata/` are local additions.

## Local changes

The local changes, in the order they were made. "Upstream PR" says whether the change
suits an upstream pull request. No upstream PR has been opened.

| Change | Why | Upstream PR |
|---|---|---|
| Initial extraction of base `24ff6fc` plus patches 01 (`messages/shared.rs`: shared-message resolution) and 02 (`fixed_array.rs`: filtered entry chunk-size width). Manifests made standalone. | Patches 01/02 are required to read the qualified HEC-RAS corpus. | 02 is superseded by upstream `c2e2e6c` (filtered chunk sizes as 1-8 byte integers). 01 is suitable. |
| Patch 03: keep exact datatype envelopes (`DatatypeMessage.raw_encoding`, `AttributeMessage.raw_datatype`), non-eager `Group::raw_attributes`, `Dataset::raw_datatype`, `Error::UnsupportedEncoding`, bounded attribute-message parsing. | The engine proves numeric and fixed-string encodings from raw bytes before decoding, and never trusts the decoded label. | Suitable as a proposal. It adds required struct fields, which is a breaking change for literal constructors. |
| Patch 04: `Dataset::raw_attributes()`. | The dataset counterpart of patch 03, used for `Units` attributes. | Suitable (additive). |
| Old-style (symbol-table) group name cache in `FileContext`; `Group::member_names`; single local-heap read; empty-heap edge case; a cache regression test that uses an external fixture. | Performance: listing large old-format groups re-read the heap per child. | Suitable (the fixture test skips when the fixture is absent). |
| Line-ending restoration only. | Keep reviewable diffs against the CRLF baseline. | No (no content). |
| Sparse row/column gather from chunks (`read_rows_columns_chunked*`), cache bypass for large reads, one index traversal for parallel cold gathers, test fixtures `tests/fixtures/perf2/*`. | Engine performance for mesh/result column reads. | Possible but engine-shaped. Discuss with upstream first. |
| Refuse unqualified non-inline extensible-array layouts as `UnsupportedEncoding`. | The non-inline EA path was not qualified against real files, so the engine fails closed. | No (a local qualification policy). |
| Bounded filter pipelines: each intermediate stage is bounded by the forward encoded-size bound of the earlier filters, and unknown or custom filters are refused under a bound. Adds test fixture `scaleoffset_deflate.h5`. | Repeated DEFLATE / scale-offset pipelines could inflate past the chunk size before the final check. | Suitable. It overlaps upstream `1638711` (filters fail instead of aborting) and needs reconciliation. |
| Ignore grid-aligned stale chunks wholly outside the current extent. | Shrunk datasets keep chunk-index entries outside the extent, and these were rejected as corrupt. | Suitable. |
| Follow soft links in old-style symbol-table groups (cache type 2 entries). | A valid soft-linked dataset in an old-format file was reported corrupt. | Suitable (upstream `main` still lacks it). |
| Soft-link resolution carries one `LinkWalk` (depth limit, active-link cycle guard, expansion budget) through intermediate path components and external links. | `/s -> "/s/x"` reset the depth on each intermediate component and overflowed the stack (195-byte file). | **Suitable**: upstream `main` has the same depth reset. |
| `OpenOptions::max_chunk_bytes` (default `DEFAULT_MAX_CHUNK_BYTES` = 256 MiB, clamped to `HDF5_MAX_CHUNK_BYTES` = 4 GiB) checked before any chunk allocation; new `Error::ResourceLimit`; every built-in filter clamped to the format maximum; DEFLATE grows its output incrementally; LZ4 refuses declared sizes its input cannot decode to. | A 4x4 dataset declaring 2^20 x 2^20 chunks made DEFLATE pre-allocate 8 TiB. | **Suitable**. The chunk limit and error are new API. The filter part overlaps upstream `1638711` (expansion-bounded reserve, fallible allocation); reconcile on rebase. |
| Checked arithmetic for file-derived sizes: `fixed_array.rs` (`FaGeometry`: closed-form paged length, checked page size and offsets, entry index range-checked); `extensible_array.rs` (saturating layout, checked offsets, placeholder entries capped at the dataset chunk count); `read_entries` sized from the remaining bytes; `global_heap.rs`, `object_header.rs` (continuation range, minimum chunk length, `get()` checksum slices), `filters/nbit.rs`, `filters/scaleoffset.rs`. | A wrapped `nelmts * entry_size` led to a 1.7 EB `Vec::with_capacity`; the paged loop ran 2^61 iterations; a global-heap add overflowed. | **Suitable**: upstream `main` still has the unchecked multiply and adds. |
| `ChunkCache::get_or_insert_with` uses a drop guard that clears the in-flight entry and wakes waiters on every exit, including unwinding. | A panicking decoder left the key in flight, and every later load of that chunk waited forever. | **Suitable**: upstream `cache.rs` is unchanged. |
| Attribute decoding checks that the declared element count fits the stored payload before allocating; fixed strings refuse zero width; string decoders reserve only what the payload holds. | A size-0 datatype let an attribute declare 2^40-2^62 elements, and `read_1d` reserved 8 TiB or hit `capacity overflow`. | **Suitable**: upstream `read_1d` still reserves the declared count. |
| `lib.rs`: the old-group cache regression test skips when `HDF5_READER_CACHE_FIXTURE` is unset instead of panicking; `Cargo.lock` committed so the crate tests standalone with `--locked`. | Lets the crate's own test suite run in CI without an external fixture. | Suitable (test only; upstream has no such test). |
| `attribute_api.rs`: `Option::map_or(true, ..)` instead of `is_none_or` (stable only since 1.82). | Keeps the crate's declared `rust-version = 1.81`; found by `cargo clippy` (`incompatible_msrv`). | Fold into the attribute element-count PR. |

## Upstream changes since the base that are not vendored

Upstream `main` has 15 reader commits after `24ff6fc` (7 are merges or release
preparation). The substantive ones:

| Upstream | Change | Status here |
|---|---|---|
| `7cbfdcb`, `7f7c65e` | Huge fractal-heap object keys and short indirect heap IDs | Not taken |
| `0cd947f` | Group members in link creation order when tracked | Not taken |
| `8430e65` | Variable-length strings with a one-byte fixed-string base | Not taken |
| `c2e2e6c` | Filtered chunk sizes read as 1-8 byte integers | Equivalent to local patch 02 |
| `e2c06e6`, `4d8aa3a` | Fallible (`try_reserve_exact`) selection and fill buffers, with regression tests | Not taken. Engine read budgets cover the engine entry points. |
| `1638711` | Filters fail instead of aborting on absurd output sizes | Not taken. The local chunk-allocation limit covers the same inputs. |

Recommended next step: rebase the vendored copy on upstream v0.9.3. Re-apply the local
changes above as a patch series, and open upstream PRs for the rows marked **Suitable**.

## Licence

The crates are dual-licensed `MIT OR Apache-2.0` upstream. The engine distributes them
under the MIT option (`vendor/LICENSE-MIT`). Both licence texts and the upstream copyright
notice are retained, and this file lists the modifications. See `THIRD-PARTY-NOTICES.md`
at the repository root for the full dependency licence bundle.
