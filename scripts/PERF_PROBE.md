# Release performance probe

This is a measurement-only harness. It uses the public engine and
vendored-reader APIs; it does not instrument or alter engine code.

## Requirements

- A Rust toolchain (see `docs/building.md`).
- Python 3.11 or later with `h5py` and `numpy`.
- One or more HEC-RAS plan HDF files with 2D unsteady results. The HEC-RAS
  example projects (for example `BaldEagleCrkMulti2D`) are a convenient public
  source. Fixture files are read in place and never modified.

## Running

From the repository root (Bash shown; adapt the variable syntax for
PowerShell):

```bash
export CARGO_TARGET_DIR="$PWD/target"
cargo build --release -p ras-commander-rust --features hdf --example perf_probe

# Fixtures may be passed with --files, or through RAS_PERF_FIXTURES
# (paths separated by ':' on Linux/macOS or ';' on Windows).
python -I scripts/perf_probe_h5py.py --out perf-results \
  --exe "$CARGO_TARGET_DIR/release/examples/perf_probe" \
  --files /path/to/Model.p01.hdf /path/to/Other.p02.hdf
python -I scripts/perf_probe_report.py --inputs perf-results --out perf-results
```

On Windows the example executable is `perf_probe.exe`.

`--discover-only` records dataset dimensions, storage/filter layouts and the
exact selected indices without running timings. The default number of
repetitions is seven (`--reps`).

## How it works

The Python driver reads fixtures with `h5py.File(..., "r")`. The Rust example
is a JSON-lines server. Modes `cold` and `reused` use the domain APIs; `raw`
uses a reused `Hdf5File` and the same coalescing strategy, bypassing RAS
validation and one-worker Rayon dispatch. A Storage decorator measures range
read time, count and bytes in raw mode without changing its `FileStorage`
implementation. Hashing uses exact little-endian numeric bytes; every timed
numeric call is hashed. Do not commit fixture bytes, executables or generated
results.

## Interpretation limits

These limits are also written into the generated report:

- Fresh sessions are not cold OS-cache tests.
- P1 returns both stored summary rows for one cell because the domain API has
  no row selector.
- M1's h5py traversal is a structural baseline, not a semantic layer
  catalogue.
- h5py retains `File` rather than `Dataset` handles.
- Internal chunk lookup, inflation, filters and numeric decoding are not
  separately exposed. The direct-reader call and storage counters bound those
  costs rather than claiming an exact internal stage profile.
- Separate warm inventory probes follow timed engine calls. Raw canonical-byte
  conversion is timed and subtracted from raw call time; engine/h5py byte
  conversion and all output hashes are outside API timings.

## Outputs

The report generator writes `perf-tables.md`, `summary.json` and
`verification.json`. Keep the raw `samples.jsonl`, `metadata.json`, build and
run logs, and a note of machine load alongside any published numbers.

## Metadata attribution probe

`perf_metadata_probe` is a supplemental attribution example. Build it with the
same release and feature flags, then pass a fixture path and an exact 2D area
name. It times individual public metadata APIs once, in one reused session, and
emits JSON lines. These sequential diagnostic observations are not
seven-repetition workload statistics and should be labelled separately in any
report. They help locate expensive catalogue subcomponents without
instrumenting engine internals.
