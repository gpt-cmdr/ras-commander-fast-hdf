# Testing and qualification

There are two levels of testing. The Rust tests in this repository check the
engine and the extension on small committed files. Qualification checks a
built wheel against h5py through ras-commander on real HEC-RAS files, and is
what allows a native path to be used by default.

## Rust tests (every change)

```bash
cargo fmt --all -- --check
cargo test --locked --no-default-features --features catalog                     # std-only build
cargo test --locked --no-default-features --features catalog,hdf,mesh,results    # engine, pure Rust
cargo test --locked --features python,exports-all,exports-pmtiles,test-hooks     # engine, extension, exports and test hooks
cargo clippy --locked --all-features --all-targets --no-deps -- -D warnings
python scripts/vendor_sources.py --check                                         # vendor/ matches PROVENANCE.json
```

Do not use `cargo test --all-features` on Linux: `extension-module` leaves the
Python symbols to the interpreter, so a test binary cannot link. The
`python,exports-all,exports-pmtiles,test-hooks` line enables everything else.
`test-hooks` compiles the panic-injection hooks (`_panic_for_test`); release
wheels leave it off, and CI's wheel smoke test checks that they are absent.

All integration tests are in `tests/`, including the former extension tests.
The committed fixtures under `tests/fixtures` are small synthetic HDF5 files (each with the script that generated it and a
manifest of SHA-256 hashes), excerpts of public HEC-RAS example results, and
malformed files that must fail with a typed error rather than crash.

### Real-file tests

Tests that need full HEC-RAS models are `#[ignore]`d by default and read their
files from environment variables:

| Variable | Points to |
|---|---|
| `RAS_NATIVE_FIXTURES` | A folder of plan and geometry HDF files (for example the HEC-RAS example projects). |
| `RAS_CATALOG_FIXTURE_ROOT` | A folder of HEC-RAS projects for the project/plan catalog tests. |
| `RAS_ENCODING_EVIDENCE`, `RAS_REVISION_EVIDENCE` | Override the committed metadata encoding and revision evidence. |

```bash
RAS_NATIVE_FIXTURES=/data/ras-examples cargo test --locked --features python,exports-all,exports-pmtiles -- --include-ignored
```

A requested fixture that is missing fails the test; it never passes silently.

### Golden HDF test corpus

[`gpt-cmdr/ras-hdf-test-corpus`](https://github.com/gpt-cmdr/ras-hdf-test-corpus)
publishes versioned, hashed HEC-RAS HDF fixtures as release assets, with
precomputed expected values (`expected/<fixture-id>/oracle.json`). The fixtures
include HEC-RAS 7.0.1 runs of the HEC examples, public eBFE/RBFS models,
RAS Mapper layers and excerpts. The `core` profile (77 files, about 151 MB
compressed) holds only HEC-RAS 7.0.1 fixtures; `legacy` holds 5.0.5–7.0 files
for best-effort checks. Fetch a profile at a pinned tag (standard-library
Python; it checks the archive against `SHA256SUMS` and every file against the
release manifest):

```bash
curl -fsSLO https://raw.githubusercontent.com/gpt-cmdr/ras-hdf-test-corpus/v1.0.1/scripts/fetch_corpus.py
python fetch_corpus.py --tag v1.0.1 --profile core --dest .corpus --expected
# optional, best effort: python fetch_corpus.py --tag v1.0.1 --profile legacy --dest .corpus
```

The files land in `.corpus/fixtures/<id>.hdf`; `.corpus/manifest.json` gives
each file's `writer`, `features`, `test_groups` and provenance. Point a test
that scans a folder at it, for example
`RAS_NATIVE_FIXTURES=.corpus/fixtures`. Tests that request specific example
files by name still need the folder they were written against. In CI, cache
`.corpus` with `actions/cache`, keyed on the tag.

## Qualifying a wheel with ras-commander

ras-commander's test suite holds the equivalence suites that compare the
native engine with h5py through the public API. Install the wheel into an
environment with a ras-commander checkout and run them in each mode.

| Suite (`ras-commander/tests/`) | Checks |
|---|---|
| `rust_route_1`, `rust_route_2`, `rust_route_3` | Every routed public `Hdf*` method on each fixture, in `h5py`, `auto` and `native` mode: identical values, dtypes, columns, index, attributes, CRS and exceptions. A guard fails the test if `native` mode quietly reads values with h5py. |
| `rust_route_extents`, `rust_route_nonhdf` | Spatial extents, and project/text readers that touch HDF indirectly. |
| `rust_fluvial_public` and `test_rust_fluvial_contract.py` | Fluvial/pluvial boundary and polygons, native against the reference. |
| `rust_mesh_quarantine` | Mesh polygons on damaged and unusual topology. |
| `rust_consumer_contract`, `rust_engine_bridge`, `rust_distribution` | The extension contract, panic containment, license bundle and an installed-wheel gate. |
| the rest of `tests/` with `RAS_COMMANDER_FAST_HDF=auto` and `=h5py` | The ordinary suite gives the same results in both modes. |

The route suites take their files from JSON manifests:

```json
{"fixtures": [{"name": "BaldEagleDamBrk.p07", "path": "/data/BaldEagleDamBrk.p07.hdf", "set": "examples"}]}
```

```bash
export RAS_ROUTE1_FIXTURES=/data/manifests/route1.json
export RAS_ROUTE2_FIXTURES=/data/manifests/route2.json
python -m pytest tests/rust_route_1 tests/rust_route_2 -n 6 -q
```

Each suite also writes a JSON-lines record per case (`RAS_ROUTE1_RESULTS`,
`RAS_ROUTE2_RESULTS`, ...), from which ras-commander's documentation table of
qualified methods is generated.

`scripts/release-gate/` holds the Windows release-gate scripts that run all of
these, plus the ordinary suite in each mode, from a pinned environment.

### What a release must show

- Every route, extents, non-HDF, fluvial and quarantine suite passes in all
  three modes, with no crashes, on Windows and Linux.
- The ordinary ras-commander suite has no failures in `auto` mode that it does
  not also have in `h5py` mode.
- The example notebooks' public calls, replayed in all three modes, give
  identical results.
- Any method newly allowed to run natively by default has at least one real
  file on which it made a native read and matched h5py exactly.

The 0.1.0 qualification covered 137 HEC-RAS 5.x–7.x files (3,800 method ×
file pairs) and 1,428 notebook calls, on Windows and Linux, with CPython 3.10,
3.13 and 3.14.

## Performance

Benchmarks are run only on a quiet machine, with each measurement in a fresh
process and the three modes interleaved. Every benchmark row also checks that
all three modes return identical data. Report whether the OS file cache was
warm; it usually is, and it favors neither mode.

## Stored-data reader qualification

`tests/data_readers.rs` exercises IC-point identities, obstruction compound records,
read budgets, stored observation tables and named steady cross-section arrays.
Its invented schema fixtures and checksum manifest are generated by
`tests/fixtures/data_readers/make_fixtures.py`; they are not hydraulic models.

`python tests/check_data_readers.py --corpus /path/to/corpus` checks the installed
wheel against both those fixtures and the checksum-pinned v1.0.1 public HDF corpus.
It compares exact numeric shape, dtype and bytes, raw obstruction compounds, and
IC-point row/name/mesh/cell identities. CI downloads core and legacy profiles using
a checksum-pinned fetch script and validates the pinned manifest and selected file
hashes before reading. It installs the wheel built by that PR, not a published
substitute. No HEC-RAS process runs and no model file is written.
