# Roadmap

Planned and in-progress work recorded after the 0.1.x release. Items here
have no committed schedule. Each item states its status: *planned* (not
started), *in progress* (a branch or draft PR exists), *blocked* (waiting on
the named dependency) or *deferred* (considered and set aside for now).

Every read-coverage item keeps the engine's refusal rule: a layout is added
only when it has been observed in real HEC-RAS files and proven identical to
h5py on them. Until then it stays refused, and ras-commander reads it with
h5py in `auto` mode.

## Next: reading HDF files directly from cloud object storage (AWS S3)

**Status:** planned, not started (recorded 2026-10-24).

**Goal.** Read HEC-RAS geometry and plan HDF files directly from AWS S3
buckets (and other S3-compatible object stores) without downloading the
whole file first. Reads should be efficient over the network: HTTP range
requests for only the bytes a read needs, with request coalescing,
read-ahead and caching of HDF5 metadata (superblock, object headers,
B-trees, chunk indexes) so a typical read costs a small, bounded number of
round trips.

**Why it matters.** Federal efforts such as FEMA's Future of Flood Risk Data
(FFRD), and state-level programs, store and process HEC-RAS models on cloud
infrastructure. A native Rust engine that also handles cloud reads could:

- replace FFRD's `rashdf` library outright, or
- become the engine `rashdf` is rebuilt around.

This is the next performance and feature step for the engine, and the one
that makes it usable in those cloud pipelines.

**Requirements to keep from the local-file engine.**

- Same results as local reads: every equivalence suite must give identical
  values for a file read from S3 and the same file read from disk.
- Same refusal discipline: anything the engine cannot prove is refused, never
  guessed. Network errors are reported as errors, never as empty or partial
  results.
- Bounded resources: limits on request count, bytes fetched and cache size,
  in the same spirit as the existing decode and allocation budgets.

**Open questions for when this starts.**

- Object-store client and authentication (credential chain, anonymous public
  buckets, S3-compatible endpoints).
- Range-request strategy and cache design, and how they interact with the
  session pool and the stored-family index.
- How ras-commander exposes cloud paths (for example `s3://` URLs) in the
  `auto | h5py | native` modes, given that h5py needs a separate driver
  (ros3 or fsspec) for the comparison path.
- Benchmarks against `rashdf` and h5py with fsspec on the same S3 objects.

## Engine read coverage

### HEC-RAS 5.x separated cross-section layout

**Status:** blocked on a real file.

Geometry HDFs written before HEC-RAS 5.0.7 store cross-section identity in
separate `River Names`, `Reach Names`, `River Stations` and `Node Names`
datasets, and their centerline and bank-line groups have no `Attributes`
table. The engine refuses this layout, so ras-commander reads these cross
sections, centerlines and bank lines with h5py. The release corpus has no such
file (its 5.0.7 files already use the compound `Attributes` table).

**Done when:** a pre-5.0.7 geometry HDF is in the corpus and the separated
family (names, lengths, bank stations, contraction/expansion coefficients,
legacy bank lines) reads identical to h5py on it.

### Non-ASCII names and paths

**Status:** planned, low priority; needs a real file.

Feature names and path attributes (for example `Terrain Filename`) stored as
ASCII fixed-length strings are refused when they contain non-ASCII bytes.
ras-commander's own writers can produce such names, and HEC-RAS probably can
too, but no corpus file has one. The likely shape is to return the raw bytes,
as `raw_fixed_strings` already does for datasets, rather than decode them.

**Done when:** a HEC-RAS-written file with accented names is in the corpus and
names and paths read byte-identical to h5py.

### HEC-RAS version coverage

**Status:** planned; needs files from the missing versions.

The qualification manifests are mostly HEC-RAS 6.6 and 7.0.1 files. Plan and
geometry files from 6.0 to 6.5 and from 7.0.2 are missing or thin, and there
are no 5.0.7 1D results (the 5.0.7 computes failed). Files from these
versions should be added and run through the equivalence suites, so that a
layout change in them is found by a test rather than by a user.

**Done when:** each added version passes the equivalence suites in all three
modes, or its new layouts are refused and listed.

### Families still read with h5py in auto mode

**Status:** planned; needs real files that contain the data.

ras-commander runs some functions on h5py in `auto` mode because no corpus
file exercises their native path (marked `h5py_until_qualified`). For four
families no file with the data was found at all: encroachments, floodway,
pump station summaries and reference summaries. On the engine side, canonical
reference summaries, the pipe network run summary, 1D reference-point
interpolation layouts and 1D summary families other than cross sections and
storage areas are still reported as unqualified.

**Done when:** for each family, a real file that HEC-RAS wrote with that data
reads identical to h5py natively; ras-commander then removes the marker.

### HEC-RAS 5.0 row-label water-surface summaries

**Status:** in progress (draft PR #9).

HEC-RAS 5.0 files label summary rows (`Row 0: ...`) instead of storing a
`Location`. PR #9 qualifies the water-surface summaries in that layout, proven
on BaldEagleDamBrk p07. Face summaries in 5.0 files (for example face
velocity) stay refused unless the same kind of proof is found for them.

### Mesh face lines and stored perimeters

**Status:** in progress (draft PR #8, stacked on #7).

`Topology::face_lines` and `read_mesh_perimeter` give Rust consumers such as
terrio mesh face polylines and stored 2D area perimeters, bit-identical to
ras-commander on 20 public files.

### Multi-part polygons with holes

**Status:** planned; needs a proven layout.

Rust consumers cannot yet get multi-part polygons with holes from the engine,
because no hole layout has been proven from real files. The perimeter
fallback through `Polygon Info/Parts/Points`, which ras-commander's Python
path uses, is also refused in the engine.

## Performance

### Share an open session across typed reads

**Status:** in progress (draft PR #6, with ras-commander PR #638).

Path-based typed reads accept an open `HdfSession`, so a public ras-commander
call opens each HDF once instead of 3 to 23 times. Cross-section reads also
stop decoding the property-table values they do not use. This closes most of
the gap on small reads that were slower than h5py; results are unchanged
(2,192 comparisons on 137 files, 0 differences).

### First-read cost of the stored-family index

**Status:** in progress (to be added to PR #6).

`stored_family_index` costs 5 to 25 ms on the first read of each file in a
process, which makes small first calls (for example `get_runtime_data`)
slower than h5py. The cost should drop without changing any refusal the index
reports.

### Remember family descriptors between reads

**Status:** deferred (not part of the PR #6 work).

Each typed read re-derives its family descriptor, about 0.2 ms per read. Small
methods make 5 to 22 such reads per call, so keeping descriptors in the
session would close more of the remaining gap to h5py on small calls.

### Identity tables on large cross-section models

**Status:** deferred (not part of the PR #6 work).

Decoding the cross-section identity table takes about 10 ms on large 1D
models (for example JUL2020). It is the main remaining engine cost of
`get_cross_sections` on large files.

### Global cap on in-flight chunk decoding

**Status:** planned.

Each worker decodes at most 256 MiB per chunk, and each call may use up to
2 GiB, but there is no cap across workers or concurrent calls. A global
in-flight decode budget would bound memory under parallel reads of hostile or
unusual files.

## Upstream and ecosystem

### Publish the engine on crates.io as one crate

**Status:** in progress.

The engine is being restructured into a single crate,
`ras-commander-fast-hdf` (library `ras_commander_fast_hdf`), for release as
0.2.0. It replaces the engine crate name `ras-commander-rust`, so the same
name is used on crates.io and PyPI. Because crates.io does not accept path or
Git dependencies, the patched HDF5 reader is carried inside the crate,
rebased on upstream 0.9.3 plus 14 patches. The Python bindings and the export
writers become optional features. Nothing is published until the maintainer
has reviewed a dry run and the re-qualification results. Rust consumers
(terrio, RustGIS) can then depend on a version instead of a Git revision.

### Upstream the hdf5-reader patches

**Status:** planned; patch set prepared, waiting for maintainer review of the PR scope.

The engine vendors a patched copy of `hdf5-reader` and `hdf5-core` from
roteiro-gis/netcdf-rust. Against upstream 0.9.3, 14 patches are still needed;
with them, crates.io 0.9.3 passes the full engine suite. Proposed grouping:

- **A. Spec-conformance read fixes:** shared-message locations (committed
  datatypes), per-stage filter-pipeline bounds, soft links in old-style
  groups, stale out-of-extent chunks.
- **B. Malformed-input hardening:** link-walk depth and cycle guard (fixes a
  stack overflow), checked arithmetic, attribute element counts, panic-safe
  chunk cache, per-chunk allocation limit.
- **C. Performance APIs** (discuss first): old-style group name cache,
  sparse row/column gathers, uncached reads.
- **D. Raw encoding evidence API** (discuss first), preferably in a
  non-breaking shape.

Two patches are CLB fail-closed policies and stay local: refusing unbounded
filters, and refusing extensible-array indexes larger than the index block.

### Report the extensible-array silent-zeros bug upstream

**Status:** planned; waiting for maintainer approval to file.

Upstream `hdf5-reader` 0.9.3 returns fill values (zeros), with no error, for
extensible-array chunks 36 to 49 of a 50-chunk h5py dataset. The engine
avoids it by refusing such indexes, which also refuses valid files. The fix
belongs upstream, in the super-block and data-block mapping; the repro file
is ready.

### Depend on hdf5-reader from crates.io

**Status:** blocked on upstream accepting the patches above.

As upstream releases take the patches, the internal copy of the reader
shrinks to the local policy patches, and eventually the engine depends on
`hdf5-reader` from crates.io. Until then the copy keeps upstream's
`MIT OR Apache-2.0` licence, both licence texts and a list of the changes.

### Reader-level limits for direct reader users

**Status:** planned.

The engine checks sizes before calling the reader, but other users of the
vendored reader do not get those checks. A zero-width float dataset still
makes the reader request terabytes for a one-element read, and the reader has
no overall read budget. A zero-width datatype refusal and a reader
`max_read_bytes` would protect them; both fit the hardening patches above.

### Drop the geo dependency

**Status:** in progress (draft PR #7).

The engine uses `geo` 0.29, which cannot share a dependency graph with the
`geo` 0.33 used by terrio 0.5 and RustGIS. PR #7 replaces it with robust
predicates that give the same verdicts (bit-identical polygons on 31 public
files).

### HEC-RAS driver for terrio

**Status:** planned; the `terrio-hecras` crate exists in the gpt-cmdr/terrio
fork, upstream PR not opened.

Upstreaming the driver to opengeos/terrio needs these engine items first: the
crates.io release, no Git dependencies, PRs #7 and #8, a minimum Rust version
compatible with terrio's (1.90), and an engine call that identifies HEC-RAS
files from the stored root `File Type` attribute, so the driver does not
claim other HDF5 files.

### In-memory byte sources and WebAssembly

**Status:** planned.

The engine and `terrio-hecras` compile for wasm32, but sessions open files by
path, so they cannot read in a browser. A byte-source session (reading from
memory or through a range-read callback) would allow WebAssembly use in terrio
and RustGIS. It shares design with the S3 reads at the top of this page.

## Platform and distribution

### Test hooks and panic messages in release wheels

**Status:** planned; waiting for a maintainer decision.

The panic-injection test hooks (`_panic_for_test` on the module and on
`HdfSession`) ship in release wheels, and a contained panic still prints
Rust's default panic message to stderr. The recommendation is to build the
hooks only behind a `test-hooks` Cargo feature that release wheels leave off,
and to install a quiet panic hook in the extension.
