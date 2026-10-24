# Stored mesh results

This page describes how the engine reads stored 2D mesh results from a HEC-RAS plan HDF:
summary arrays (such as maximum water surface), time series per cell or face, and
boundary-condition line series. It explains how each variable is assigned to a cell, face
or other domain, how selections and byte budgets work, what reductions return, and which
inputs are deliberately refused. The surface was added in engine API 5 and extended in
API 6 (session caching) and API 9 (legacy absent-Location series).

## Entry points

`hdf::mesh_results` is available with `hdf` alone. The session methods are
`mesh_summary_inventory`, `read_mesh_summary`, `mesh_series_inventory`,
`read_mesh_series`, `mesh_boundary_inventory`, `read_mesh_boundary_series`,
`reduce_mesh_series` and `run_summary`. These use the canonical unsteady
Base Output block; other blocks are not silently substituted.

## Inventories and domain assignment

Inventories distinguish absent canonical groups (Unavailable) from present
groups. Each numeric variable has its own capability. Supported inventory
means enumeration succeeded, not that every variable is supported. Domain
assignment requires a qualified stored scalar Location attribute:
`Cells` maps to Cell, `Faces` maps to Face, and the observed singular `Cell` alias maps
to Cell (Davis/Glaise cumulative precipitation/infiltration/excess arrays). No variable
name, Columns label, unique size or coincident count chooses the domain. Only the
chosen domain's geometry count must be present and match; missing/unknown Location or
missing count is Unsupported(identity_layout), a known Location/count contradiction
is Corrupt. Facepoint domains without a qualified Location token remain unqualified.
Non-rank-two series are Unsupported(schema); mismatched Time rows remain Corrupt.
This rule also applies to summary variables. Summaries and unknown-name series lacking Location
(such as many Maximum/Minimum Water Surface arrays) are listed with explicit refusal;
read_numeric retains their literal values without attaching a domain. Existing generic
read APIs are not silently relabelled as typed mesh results.
BC columns are local column positions, domain Other; exact Columns/Cells/Faces
attributes retain labels and any global IDs. They are not inferred from names.

## Values and selections

All numeric types supported by the generic reader stay exact, including f32,
f64 and integer counters. Summary selections return all stored rows, including
three-row starting-difference datasets and rank-one counters. The descriptor
marks time_of_extreme_row only where two stored rows explicitly label the
second as Time. Every typed attribute (including array units and row labels)
retains raw datatype and payload evidence. Timestamp conversion of summary
offsets is not implemented: time rows remain exact stored numeric values.

Selections retain unsorted and repeated element IDs. Contiguous selections use
one hyperslab; sparse selections coalesce runs within a storage column chunk
when the resulting slab fits the remaining working-byte allowance. Output is owned and row-major. Identity
uses source file, dataset path and row zero; elements carry the selected dataset
column identities and rows carries absolute stored row indices. BC selections
refer to local columns, not the global IDs that attributes may declare.

## Budgets

`metadata_max_bytes` bounds each raw metadata object consulted. `max_bytes`
bounds the combined returned numeric buffer, element-ID buffer and one live numeric slab. Sparse slabs scatter immediately; no collection of slabs is retained. This is not a combined peak-RSS budget: HDF whole-chunk
inflation, cache, metadata and output copies are separate storage costs.
Reduction bounds combined returned values and index bytes before allocation.
It streams one selected time row at a time; repeated chunk decompression is
possible. No reduction performance claim is made. A future chunk-row batching
optimization requires profiling and exact equivalence checks.

## Reduction

Reduction returns minimum then maximum, with absolute time indices, retaining
F32/F64. First equal value wins (including signed zero); first NaN propagates
and its index wins, matching numpy argmin/argmax. Values retain the bits at those selected indices; numpy min/max may instead return a later signed zero. No dry, threshold or
NaN-removal policy is imposed. Empty time windows are InvalidRequest. Integer
reduction remains Unsupported(encoding). Empty ordinary slab selections work.

## Run summary

Run summary recursively reports typed raw attributes at Results/Unsteady/Summary
and its Volume Accounting subtree. It does not decode volume-accounting datasets
or infer compute success. The hierarchy has explicit depth and group-count caps.

## Not implemented

Not implemented: decoded summary civil times, summary siblings
for 1D/structures, nested per-area computation/log datasets, non-Base output
blocks, semantic BC label/element-reference validation, and a dedicated scalar
unit field for summary arrays. Exact typed attributes are available now.

## Oracle tests

Generate independent oracles with `tests/fixtures/mesh_results_oracle.py PROBE.json
OUTDIR`, where `PROBE.json` is a list of `{"path", "exists", "sha256"}` records for the
source plan HDF files. Set `RAS_WAVE8_RESULTS_ORACLE` to that output directory for the
opt-in integration tests. The manifest pins original source
hashes, deterministic unsorted samples (first/middle/last row, up to 64 distinct
elements plus a repeat, including the final element), full summary hashes,
and five-row numpy reduction results. edge.hdf supplements real files with
NaN/tie, malformed orientation and invalid-selection cases. An unset variable fails the opt-in test with an explicit prerequisite message.

## Session reads and caching (API 6)

Session reads memoize per-variable descriptors by metadata budget. BC reads
qualify the requested line, sharing the memoized time axis; unrelated BC siblings
are not decoded. `read_mesh_boundary_batch` accepts all lines (`None`) or an ordered
named subset. Its byte budget covers returned numeric arrays and element IDs over
the whole batch. `read_mesh_summary_values` applies the same validation/budget as
the identity-bearing summary but returns only values.

`with_cache_options` configures retained chunk bytes/slots (zero bytes disables
retention). `chunk_cache_stats` exposes counters. Full reads larger than the cache
bypass retention and hot-array copies; chunk decoder scratch remains. Chunked 2D
sparse reads gather selected raw elements directly from decoded chunks, preserving
strides, order and duplicate columns. The output budget is not an RSS ceiling:
metadata, bounded retained chunks and decoder scratch remain additional memory.

Requested-only BC reads (including named subsets) return valid
requested data even when an unrelated sibling line is corrupt. Inventory and an
all-lines batch still report the corrupt sibling. A zero-byte cache deliberately
disables the optional 32 MiB hot full-array copy as well as decoded-chunk retention:
retaining that copy would defeat the caller's explicit no-retention policy.
Non-inline extensible-array chunk indexes are conservatively refused until their
block mapping is qualified; earlier versions could silently return fill values
for allocated chunks beginning at index 36. This refusal applies to full and
bounded reads. Inline-only EA indexes remain supported.

The hot full-array copy is limited to 32 MiB **per dataset**. A session has no
limit on the number of eligible datasets retaining such a copy and no aggregate
hot-array byte budget. These copies are additional to the configured chunk cache;
a Python session LRU bounds open files, not retained datasets within each file.
A session-total hot-array budget or LRU is not implemented.

For the embedded reader's old-group cache regression test, set the cache fixture from the
repository root (the test is skipped if it is unset):

```powershell
$env:HDF5_READER_CACHE_FIXTURE = (Resolve-Path tests/fixtures/hdf_numeric/numeric_types_v1.h5).Path
cargo test --manifest-path vendor/reader-tests/Cargo.toml
```

```sh
HDF5_READER_CACHE_FIXTURE="$PWD/tests/fixtures/hdf_numeric/numeric_types_v1.h5" cargo test --manifest-path vendor/reader-tests/Cargo.toml
```

## Absent-Location legacy mesh series (API 9)

HEC-RAS 5.0.7 output (JUL2020_D100RAS.p04, area D601) stores Depth, Water Surface,
Face Velocity and Face Shear Stress without Location. Only when Location is **absent**,
a rank-two numeric dataset at exactly
`Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series/2D Flow Areas/<area>/<name>`
qualifies by this immutable exact-name table (no prefix, substring or case folding):

| Name | Domain |
|---|---|
| Depth | cell |
| Water Surface | cell |
| Face Velocity | face |
| Face Shear Stress | face |
| Face Area | face |
| Face Manning's n | face |

This mirrors ras-commander `_LEGACY_MESH_SERIES_DOMAINS` (ras-commander PR #565). A present Location
stays authoritative: an unknown, non-scalar or non-text value refuses
(Unsupported(identity_layout)) and never falls back to the name; a valid one decides the
domain even against the name. The domain axis must equal the stored geometry count
(Cells Center Coordinate rows for cells, Faces FacePoint Indexes rows for faces, ghosts
included); a mismatch is Corrupt and an absent count Unsupported(identity_layout). For
cell series on either domain path (this legacy rule or a stored `Location` of `Cells`/`Cell`),
a physical `Cell Count` recorded in `Geometry/2D Flow Areas/Attributes` for the exact area
Name (finite and > 0, truncated, last duplicate row wins; an absent table or column records
none) above the stored cell rows is Corrupt, as in PR #565. Summary arrays and BC columns
are not bounded. Rows
check against the block Time as for every series (mismatch Corrupt). Unknown names,
rank-one time-only series and every Summary Output array are unchanged.

`MeshResultVariable.domain_rule` (`DomainRule`, `as_str()`) records which rule produced
`domain`: `stored_location`, `legacy_exact_name`, `labelled_summary`, `boundary_columns`
or `unqualified`. It is kept when the capability later refuses the domain (for example a
legacy count mismatch). The values are the literal stored values; the rule never
transforms them. Real excerpts and mutations (unknown names, wrong counts, rows mismatch,
present-but-invalid Location) are in `tests/fixtures/legacy_location`
(`generate_legacy_location_excerpts.py`, `tests/legacy_location.rs`). The regenerated
seven-file oracle (`fixtures/mesh_results_oracle.py`, now with the same rule and a
`domain_rule` per variable) has 238 qualified variables and 39 refusals: exactly the four
JUL D601 series moved from refused to qualified, nothing else changed. Frozen pre-API-9
oracles encode the earlier refusal and no longer pass for those four series.

### Time dataset requirement and intentional strictness

Cell/face series on either domain path also require the Base Output
`Unsteady Time Series/Time` dataset itself, present, rank one and proven numeric, as PR #565
does. An axis rebuilt from `Time Date Stamp`/`(ms)` alone no longer qualifies a series:
absent `Time` is Unsupported(schema) ("Base Output Time dataset absent; date stamps alone do
not qualify mesh series"), and a non-rank-one or unproven `Time` is Unsupported(schema).
Boundary-condition columns and summaries are unchanged. A legacy series whose geometry count
is absent reports "geometry count for legacy exact-name domain absent".

Intentional divergences from #565, all typed refusals (never a different value); none occurs
in the qualification corpus, and the Python bridge falls back to h5py in auto mode:

| Input | #565 | Engine |
|---|---|---|
| variable-length (vlen) string `Location` | accepted (`Cells`/`Cell`/`Faces`) | Unsupported(identity_layout): vlen metadata is not typed exactly |
| big-endian or `f2` series | qualified | Unsupported(encoding) |
| integer `Time` | qualified | inventory error Unsupported(encoding) (Time must be f32/f64) |
| `Cells Center Coordinate` / `Faces FacePoint Indexes` / `FacePoints Coordinate` not `(N, 2)` | refuses only the affected domain | inventory-wide Corrupt ("invalid mesh identity dimensions") |
| non-compound `Geometry/2D Flow Areas/Attributes` | ignored for the physical count when no `Cell Count` field | cell series Unsupported(schema) |
| `Attributes` `Name` column not proven fixed text (variable-length or non-ASCII in ASCII) | decoded | cell series Unsupported(encoding) |
| rank-one series without Location | time-only `(time,)` | Unsupported(schema) (out of scope by spec) |

Mutation evidence: of 1,305 independently generated mutant series, the engine and an independent #565
reference agree on 1,252; all 53 differences are rows of this table (the engine refuses),
none is engine-permissive, and all 526 series both sides qualify have equal payload SHA-256.

## Label-qualified stored-cell summaries

When Location is absent, a dataset under the canonical Base Output/Summary Output/
2D Flow Areas/<area> namespace can qualify as a stored-cell summary only with
rank two, two rows, exact Rows Variables [WSEL, Time], exact Units [ft or m, days],
and columns equal to Geometry/2D Flow Areas/<area>/Cells Center Coordinate rows.
That stored count includes ghost rows; physical Cell Count never substitutes and
no clipping occurs. Names such as Maximum or Minimum do not select the domain or
infer an extremum. Values and the labelled Time row remain literal source data.
Present Location still uses the primary Location rule; conflicting/unknown values
are not bypassed. Unlabelled JUL extrema and iteration/difference summaries remain
refused; the four JUL series qualify through the API 9 legacy rule above. Current
seven-file oracle: 238 qualified variables, 39 explicit refusals (234/43 before API 9),
with generic literal checks preserved for every refusal.
