# Engine conformance

This page records how the engine's core readers were qualified: which checks run in every
build, which real HEC-RAS files were compared against independent Python (h5py/NumPy)
oracles, and how to rerun the opt-in real-file tests. It does not claim complete HDF5 type
or layout coverage, and it does not make performance claims. Read families added later
are covered in [read-families.md](read-families.md), [mesh-results.md](mesh-results.md)
and [one-d.md](one-d.md).

## Portable checks

The portable suite runs in the default engine test gate
(`cargo test --locked --no-default-features --features catalog,hdf,mesh,results`). When this
baseline was qualified it comprised 18 unit tests plus 6 catalogue and 6 HDF portable
integration tests; the suite has grown since. Real-file tests are marked `#[ignore]` and
run only when requested.

Numeric safety fixtures are small synthetic HDF files checked into this repository. 76
dataset cases run across serial and parallel reads and 1- and 4-thread configurations,
checking exact typed values, shape and bytes, or an explicitly expected unsupported
result. They include ten numeric types, large `u64` values, encoding, byte order and
precision, filters and fill values, scalar and empty arrays, owned output lifetime,
bounded thread counts and huge-stride handling.

Formatting and strict all-feature, all-target Clippy pass. The catalogue-only normal
dependency graph contains only this crate. The engine's own code forbids unsafe Rust; the
vendored generic reader retains its separately licensed implementation.

## Real geometry and results

Opt-in real tests read immutable stored HEC-RAS 5.0.7, 6.6 and 7.0 files and compare them
with frozen independent h5py/NumPy/Python-polygonize oracles. Rust test execution does not
call Python. Whole inputs are SHA-256 checked before and after each run.

| Fixture | Mesh polygons | Scope |
|---|---:|---|
| `JUL2020_D100RAS.p04.hdf`, 5.0.7 | 340,246 | Two invalid cells (317422 and 339176) are explicit quarantine omissions; strict mode refuses invalid geometry; ghost records remain distinct |
| `BaldEagleDamBrk.p07.hdf`, 6.6 | 87,039 | Exact canonical coordinates and source/composite IDs |
| `BaldEagleDamBrk.p18.hdf`, 7.0 | 4,425 | Both meshes and repeated local IDs retained |

All three mesh workflows pass with 1 and 4 threads. Full water-surface temporal values,
first peaks, finite counts and strided reads match independent frozen digests at both
thread counts. Stored shapes retain ghost columns where present: 193 x 352141, 865 x 89879,
and 865 x 3947 plus 865 x 1251. Do not confuse these with physical-cell or valid-polygon
counts.

Real metadata tests read the stored `File Version` and fixed `Time Date Stamp` strings;
raw synthetic attributes retain 60 payloads. This baseline is not typed compound or
reference decoding, and not native CRS resolution (both are covered by later families).

Run with a local fixture directory containing the named files:

```text
RAS_NATIVE_FIXTURES=/local/fixtures cargo test --locked --no-default-features --features catalog,hdf,mesh,results --test hdf_conformance --test mesh_conformance -- --include-ignored --test-threads=1
```

In PowerShell, set `$env:RAS_NATIVE_FIXTURES` before the Cargo command. Missing requested
inputs fail rather than passing as empty coverage. Debug-build qualification timings are
not release performance measurements.

## Real project and plan catalogue

Catalogue qualification comprises 6 portable tests and one opt-in real test covering 11
cases, 10 distinct plan paths and 7 project paths. Frozen Python and raw-byte comparisons
cover declared program versions 5.00 through 6.70, relative, case-varying and slash
links, missing and malformed listed plans, steady, unsteady and quasi-unsteady sources,
byte retention, local civil dates (including year 3000 and hour 24) and interval
semantics.

An archive directory labelled 7.0 contains an older declared plan version, so genuine
7.x-declared text remains unqualified. The presence of an HDF path in a catalogue is
availability metadata, not proof of a successful computation.

```text
RAS_CATALOG_FIXTURE_ROOT=/local/text-corpus cargo test --locked --no-default-features --features catalog --test catalog_conformance -- --include-ignored
```

Catalogue source bytes and declared times are preserved; no Python, HDF or GIS runtime is
required for this feature. Encoding scope is UTF-8 (with or without BOM) and a named
Latin-1 fallback, not all legacy Windows code pages.

## Review fixes

API review of this baseline found, and the engine now handles: huge-stride overflow and
widening, unbounded thread requests, and malformed datatype verification being
classified as unsupported rather than corrupt. The portable and real runs qualify the
revised code. Generic reader provenance and patches are in `vendor/` (see
[provenance.md](provenance.md)).

## Remaining gaps in this baseline

At the time of this baseline the following were out of scope; several are now covered by
later families (see [engine-api.md](engine-api.md)): native CRS evidence and fallback;
typed compound, variable-length and reference records and a broader metadata matrix;
declared physical-count and layer descriptors; the 5.0.3 names-only layout; 1D, steady,
reference, connection and pipe families; timestamp and time-zone interpretation; range
sources and cancellation; a platform/MSRV matrix. Tests overlap, so no whole-library
coverage percentage is derived from their counts.

## 1D and steady baseline

`one_d_conformance` runs two opt-in tests. Independent h5py oracles cover Muncie geometry
(61 cross sections), Balde Eagle Creek geometry (178 cross sections) and the Nutrient
Example steady plan (18 cross sections, one stored profile, water surface and flow). The
sources were extracted with `RasExamples` from the official HEC-RAS 6.6 example archive;
declared HDF versions are recorded separately from archive labels. Both 1- and 4-thread
configurations compare every source-order identity, name and bank field; bank values
compare exact widened IEEE bits, and selected numeric results compare exact dtype, shape
and byte SHA-256. Whole-source SHA-256 is checked before and after. No Python is invoked
during Rust tests.

Eight supplemental files test steady and unsteady selected reads and `f32`/`f64`
ownership, missing results or identity, wrong dtype, incompatible cross-section shape,
empty and variable-length profile names, and invalid selections. The 6.5 and 6.6
archives each shipped only the Nutrient steady computed plan, and 7.0 shipped no plan HDF,
so computed real unsteady 1D conformance was a gap in this baseline; geometry from an
unsteady project does not fill it. No HEC-RAS execution was performed. Later 1D work is
described in [one-d.md](one-d.md).

Run with `RAS_1D_FIXTURES` pointing to the extracted sources and `RAS_1D_EVIDENCE` to the
frozen oracles and supplemental files:

```text
cargo test --locked --no-default-features --features catalog,hdf,mesh,results --test one_d_conformance -- --include-ignored
```

Default test runs ignore external-input tests; missing requested inputs fail. This
baseline does not claim complete result-family or platform coverage.
