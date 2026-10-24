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
cargo test --locked --features python,exports-all,exports-pmtiles                # engine, extension and exports
cargo clippy --locked --all-features --all-targets --no-deps -- -D warnings
cargo test --locked --manifest-path vendor/reader-tests/Cargo.toml               # embedded reader, upstream tests
python scripts/vendor_sources.py --check                                         # vendor/ matches PROVENANCE.json
```

Do not use `cargo test --all-features` on Linux: `extension-module` leaves the
Python symbols to the interpreter, so a test binary cannot link. The
`python,exports-all,exports-pmtiles` line enables everything else.

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
