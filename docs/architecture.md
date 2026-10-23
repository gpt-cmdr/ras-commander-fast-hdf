# Architecture

## Three layers

```
ras-commander (Python, separate repository)
  Hdf* classes ── public API, unchanged
  ras_commander/fast_hdf.py ── routing: auto | h5py | native, sessions, fallback
        │
        ▼
ras_commander_fast_hdf (python/, PyO3 extension)
  converts engine results to NumPy arrays, dicts and Python exceptions;
  native kernels: mesh polygons, fluvial/pluvial, PMTiles and vector exports, RAS text
        │
        ▼
ras-commander-rust (crates/ras-commander-rust, pure Rust library)
  HEC-RAS knowledge: project/plan catalog, HDF layouts, identities, units, time, CRS evidence
        │
        ▼
hdf5-reader / hdf5-core (vendor/, pure Rust HDF5 decoding)
```

- **The engine** (`crates/ras-commander-rust`) knows how HEC-RAS stores things:
  which datasets and tables exist in each HEC-RAS version, how mesh topology is
  laid out, how cross-section identities, time stamps and units are recorded.
  It has no Python in it, so other Rust programs can use it directly.
- **The extension** (`python/`) is the only place Python appears. It turns
  engine results into NumPy arrays and plain Python objects, maps engine errors
  to Python exceptions, contains panics, and adds kernels that only
  ras-commander needs.
- **ras-commander** decides, for each read, whether to use the extension, and
  builds the same pandas, xarray and GeoPandas results from either source.

## How a read flows

1. A public ras-commander method (for example
   `HdfResultsMesh.get_mesh_timeseries`) asks `fast_hdf` for a dataset instead
   of opening it with h5py.
2. `fast_hdf` checks the mode (`RAS_COMMANDER_FAST_HDF` or the method's
   `engine=` argument) and whether this method is qualified for native reads.
3. In `auto` or `native` mode it calls the extension, inside a session that
   lives for the duration of the public call.
4. The extension calls the engine, which validates the layout, budgets the
   read, decodes it and returns typed values or a typed error.
5. On success, ras-commander builds its result exactly as it would from h5py.
   On a refusal, `auto` repeats the read with h5py; `native` raises
   `FastHdfError`.

## Exact or refuse

The engine's central rule: return exactly what h5py would return, or refuse
with a typed reason. It never guesses an encoding, a CRS, a unit or a layout,
and never repairs data. Because `auto` falls back to h5py on every refusal, a
refusal costs only speed, while a wrong value would be a silent error in an
engineering result. When in doubt, the engine refuses.

Refusals are reported with one status vocabulary (see [Rust engine](engine.md)):
`unavailable` (canonically absent), `unsupported` (present but not proven, with
a code such as `encoding` or `resource_limit`), `corrupt` (damaged or
contradictory) and `delegated` (checked by another reader).

## Qualification

A native read path is used by default only after it has been proven identical
to h5py on real files. ras-commander's equivalence suites run every public
method on a corpus of HEC-RAS 5.x–7.x files in all three modes and compare
values, dtypes, columns, index, attributes, CRS and exceptions. Methods the
corpus has not exercised natively are marked in ras-commander and stay on h5py
in `auto` mode. See [Testing and qualification](testing.md).

## Safety

- The engine is `#![forbid(unsafe_code)]`. The vendored HDF5 reader uses
  `unsafe` only for read-only memory mapping and a few decoding paths.
- Every count, offset and size read from a file is checked before use, and
  release builds keep integer-overflow checks on.
- Every read is budgeted before allocation (2 GiB of decoded values per call by
  default). Larger reads are refused; ras-commander then reads them in chunks.
- A panic that escapes despite this becomes an ordinary Python exception
  (`python/src/panic_guard.rs`), so `auto` falls back instead of the process
  crashing.
- Files are opened read-only. Sessions are closed when the public
  ras-commander call returns, so no file stays locked on Windows.

## Compatibility handshake

The extension exports `API_VERSION`. ras-commander uses the extension only
when that number equals the one it was built against; otherwise it logs one
warning and uses h5py for everything (or raises in `native` mode). This lets
the two packages be upgraded independently without ever mixing incompatible
contracts. See [Releasing](releasing.md).
