# Python extension contract

This page is the detailed contract of the compiled Python module `ras_commander_fast_hdf`
(the `ras-commander-fast-hdf` wheel): how it is built and identified, every group of
bindings and the shape of what they return, how errors and panics reach Python, and which
licences the wheel ships. The module is an internal building block: applications normally
use ras-commander's `Hdf*` classes and its `fast_hdf` adapter, which decide between this
module and h5py. The current binding contract is `API_VERSION` 11, built against engine API
10 from this repository. An overview is in [../python-extension.md](../python-extension.md);
the [API version history](#api-version-history) at the end lists what each version added.

## Build and identity

The extension and the engine are one crate, `ras-commander-fast-hdf`; the extension is its
`python` feature (sources in `src/python/`). maturin builds the wheel from
`python/pyproject.toml`, which points at the root `Cargo.toml` and enables
`python,extension-module`. The main ras-commander package remains installable without a
Rust compiler; wheels are built by CI (see [../releasing.md](../releasing.md)). To build
locally, use a Rust toolchain with an installed platform linker and Python 3.10 or later:

```text
cd python
uv tool run --from maturin maturin build --release --locked --out ../dist
uv pip install --python <your-python> <resulting-wheel>
cd ..
cargo test --locked --features python,exports-all,exports-pmtiles
```

Set `CARGO_TARGET_DIR` to a local directory rather than placing build outputs on a network
share. Windows needs the Visual Studio C++ Build Tools. `PYO3_PYTHON` can select the
interpreter explicitly. The wheel uses the CPython 3.10 stable ABI; cross-platform
compatibility is tested per platform. `extension-module` is enabled by maturin, while
`cargo test` with `python` (and without `extension-module`) links Python normally. The
minimum supported Rust version is 1.88.

| Name | Meaning |
|---|---|
| `API_VERSION` | Binding contract version (11). One Rust constant; ras-commander requires an exact match. |
| `TEXT_API_VERSION` | Version of the RAS text-file bindings (1). |
| `ENGINE_COMMIT` | Commit of this checkout the wheel was built from (`git rev-parse HEAD`), or the 40-digit value of `RAS_ENGINE_COMMIT` when set (for builds from a source archive), else `"unknown"`. |
| `ENGINE_FEATURES` | Engine Cargo features enabled in the build, in a fixed order: `("catalog", "hdf", "mesh", "results")` (the `python` feature enables all four). |

The root `build.rs` sets `ENGINE_COMMIT` and `ENGINE_FEATURES` for the `python` feature. It
asks Git only when the package directory is itself a Git checkout, and rejects a
`RAS_ENGINE_COMMIT` that is not a full 40-digit commit. `Cargo.lock` freezes resolution.

`capabilities()` returns the engine commit and feature list, `engine_api_version`,
`engine_capabilities` (the engine's compiled features and owner/name/feature entry points),
`default_max_bytes`, and `bindings`: the sorted set of actually registered functions and
classes (including the `NativeAnalysis` class, excluding exception classes), so optional
feature builds report only what was registered. ras-commander's tests check this set
against an explicit expected list (79 entry points at API 11 from engine 0.2.3, 78
before; including the `_panic_for_test` hook).

## Core numeric, mesh and tile functions

```python
read_numeric(path, dataset_path, selection=None, threads=None, parallel=True)
read_numeric_batch(path, dataset_paths, threads=None, parallel=True)
assemble_cells(face_points, face_endpoints, face_perimeter_info,
               face_perimeter_values, cell_face_info, cell_face_values,
               threads=None, *, strict=True, return_diagnostics=False)
write_pmtiles(layers, output, min_zoom, max_zoom, threads=None)
```

File arguments accept `str` or `os.PathLike`.

### Numeric reads

Numeric reads return NumPy arrays with their native numeric dtype and original rank.
Supported types are signed/unsigned 8/16/32/64-bit integers and 32/64-bit floats in native
byte order. Other datatypes and non-native byte order raise `UnsupportedError`; use h5py
for those. A selection is a list of nonnegative `(start, stop, step)` triples, one per axis,
with positive steps and exclusive stops; bounds must already be normalized. Integer axis
squeezing and arbitrary advanced indexing belong to the Python adapter. Batch reads reuse
one file context and return a dictionary keyed by dataset path; repeated identical paths
collapse to one key (the last returned value wins). Arrays own their memory after return
and do not depend on an open HDF handle.

Since engine API 10, `read_numeric`, `read_numeric_batch` and `read_topology` (used by
`HdfSession` numeric reads and the reader) refuse more than `DEFAULT_READ_MAX_BYTES`
(2 GiB) of decoded values before allocating. `auto` mode falls back to h5py for such reads;
required-native (`native`) mode raises `FastHdfError`. The engine's additive `*_bounded`
variants are not bound yet.

### Mesh cell assembly

Mesh inputs are two-dimensional NumPy arrays: coordinates are `float64` and
indices/offsets are `int64`. Coordinates have two columns. Face endpoints and perimeter
info have at least two columns; cell face info has at least two; cell face values have at
least one (face ID). The kernel accepts strided arrays and copies inputs before releasing
the GIL, protecting against concurrent mutation of borrowed Python memory. Output is
`(coordinates, offsets, cell_ids)`: `float64` coordinates of shape `(N, 2)`, `int64` offsets
of length `C+1`, and `int64` original cell IDs of length `C`. Each offset interval contains
one closed ring.

Assembly follows face endpoint connectivity, reversing complete face chains when
necessary. Every intermediate perimeter coordinate is retained. Zero-face cells and
one-open-face boundary ghost cells are omitted without renumbering. Disconnected,
branching, self-loop or unclosed topology raises `TopologyError`; the Python compatibility
path can polygonize such cases. This kernel handles simple single-ring cell boundaries,
not arbitrary polygonization or automatic geometry repair.

Native diagnostics (`return_diagnostics=True`) are dictionaries with
`subject`/`message`/`capability` plus context fields; cell omissions add `cell_id`/`kind`.
The ras-commander adapter preserves its public polygon diagnostic records and exception
types. Engine string-only diagnostics carry an explicit unavailable capability whose
reason states that typed capability evidence was not supplied; prose is never classified.

Since engine API 10, mesh cell polygon rings follow the ras-commander Python contract
(clockwise, starting at the first listed face), so `mesh_polygons` and the Python
`HdfMesh.get_mesh_cell_polygons` agree byte for byte (see
[engine-api.md](engine-api.md#ring-order-api-10)).

### PMTiles

`write_pmtiles` takes a list of dictionaries with keys:

* `name`: unique nonempty layer name.
* `coordinates`: `float64` `(N, 2)` WGS84 longitude/latitude coordinates.
* `offsets`: `int64` ring offsets, length `C+1`.
* `cell_ids`: `int64` nonnegative IDs unique within the layer.
* `properties`: dictionary of column names to Python lists of length `C`.

Property values are strings, signed 64-bit integers, floats, booleans or `None`.
Non-finite property floats become null; out-of-range integers raise. Coordinates must be
finite and within Web Mercator latitude limits. pyproj transforms the packed coordinate
arrays outside this kernel; no GeoJSON, WKB or GeoDataFrame serialization is required for
the native tile path.

The exporter writes MVT PMTiles, zooms 0-22, with dropping, clustering, coalescing and
presimplification disabled. Clipping and MVT quantization still apply. The pinned
freestiler encoder replaces a collapsed exterior with a one-pixel square at its centroid,
preserving visibility rather than exact subpixel geometry. PMTiles are a visualization
product; packed source polygons remain the analytical geometry. Source features remain in
memory; upstream freestiler spools encoded tile chunks to temporary storage. The Python
adapter supplies a staged output path and atomically publishes it after success; the
underlying PMTiles writer also stages archive assembly before replacing its destination.

The native tile orchestrator uses freestiler's assignment, clipping, MVT, compression and
PMTiles writer kernels, inserting an integer-grid topology repair before compression. Real
BaldEagle exports exposed rounding-induced backtracking spikes and self-intersections
despite valid source polygons. Simple single-ring features bypass overlay; pathological or
multipart rings are reconstructed with `i_overlay` using nonzero winding. Splits remain
parts of the same feature with unchanged IDs and property tags. Repair operates on the
final tile grid, avoiding a second projection or quantization. Processing uses batches of
32 tiles and spools compressed output. Eight decoded real failures, with source
mesh/cell/tile metadata, are kept as compact Rust regression fixtures in
`tests/fixtures/python/quantized_regressions.json`.

## Threads

Expensive HDF reads, topology assembly and tiling release the GIL. Thread pools are reused
by effective thread count; at most eight configurations are cached. The default is the
smaller of eight and the available CPU count. Explicit `threads` values must be 1-1024.
Outer parallel workflows should set a smaller count to avoid oversubscription. Serial
numeric reads use `parallel=False`. Structure reads use `threads=None` and the shared
bounded reader thread validation. Sessions are described in [sessions.md](sessions.md).

## Errors

| Engine error | Python exception |
|---|---|
| NotFound (object) | `KeyError` |
| FileNotFound | `FileNotFoundError`, with filename and `errno=2` |
| Io | `OSError` |
| Unsupported | `UnsupportedError(ValueError)` |
| InvalidRequest | `ValueError` |
| Corrupt | `CorruptDataError(ValueError)` |
| StructuralTopology / InvalidGeometry | `TopologyError(ValueError)`, `mesh_structure_error` True / False |
| Internal / future kind | `RuntimeError` |

These exception types are exported by the extension. One central mapper, shared by the
reader and mesh bindings, attaches `engine_kind` (the engine's snake_case `as_str()`),
`engine_code` (the optional unsupported code), `subject` and `source_path`. Exception text
includes the engine kind prefix; callers should catch exception types rather than parse
strings. Annotation failure preserves the original exception rather than replacing its
category. Automatic fallback must never catch every exception indiscriminately or hide
corrupt inputs. ras-commander's public `fast_*` adapter failure policy and `FastHdfError`
wrapping are unchanged.

### Panic containment

Every `#[pyfunction]` and `#[pymethods]` body runs inside `panic_guard::guard`
(`catch_unwind`). Without it, a Rust panic would surface as PyO3's `PanicException`, a
`BaseException`, which escapes an `except Exception` h5py fallback.

- With `ras_commander.fast_hdf` imported, a panic raises its
  `FastHdfPanicError(FastHdfError)`: `auto` mode falls back to h5py, and `native` mode
  reports a `FastHdfError`.
- The standalone module raises `NativePanicError(RuntimeError)`.

`HdfSession` catches engine panics while it holds its lock, and recovers a poisoned lock. A
panic therefore never disables a session: later reads, `closed` and `close()` keep working,
including on process-cached sessions.

`_panic_for_test(message)` and `HdfSession._panic_for_test()` inject a panic for tests
(used by ras-commander's `tests/rust_engine_bridge/panic_guard_test.py`).

Aborts (stack overflow, allocation failure) cannot be caught in-process. The engine
prevents those on file input with read budgets and malformed-input hardening (see
[engine-api.md](engine-api.md#byte-budgets) and `vendor/hdf5-reader/PROVENANCE.md`): bounded
soft-link resolution, decoded chunk size bounded before allocation (256 MiB default),
checked size arithmetic, chunk-cache recovery after a decoder panic, and attribute element
counts checked against their payload.

## Wire conventions

These apply to every typed binding below.

- Module functions are path-first. `max_bytes` and `metadata_max_bytes` default to the
  engine's `DEFAULT_MAX_BYTES` (67108864, 64 MiB); defaults are compile-time checked against
  the engine constant. They keep the engine meaning (each raw stored object, or the selected
  output), not an aggregate memory limit; the engine's 8 MiB profile/packed-name exception
  remains.
- Capability dictionaries have exactly `status`/`code`/`reason`, using the engine
  `as_str()` spellings.
- Evidence dictionaries (`Reported<T>`) contain `kind`/`value`/`raw_bytes`/`capability`, or
  `{capability, value}` for typed values. `kind` describes evidence, not a competing
  availability status. Raw bytes may be `None` where the engine supplies none.
- Identities are `{source, table, row}` plus existing domain names; compatibility
  `index`/`source`/`path` aliases remain where already exposed.
- Enum strings come from engine methods. Structure kind preserves `f32`/`f64` wire names,
  while `dtype` uses the common NumPy dtype spelling.
- Compounds are `{shape, raw_bytes, columns}` with exact stored bytes and typed column
  metadata; Python scalar lists are decoded views, not float-bit oracles.
- Numeric arrays keep their stored NumPy dtype; buffers move into NumPy without float
  widening. Selections retain element order and duplicates. Read results are owned NumPy
  arrays; other reports use plain dict/list/bytes values.
- Floating metadata preserves Python float NaN and infinity (not coerced to null), so these
  values are not strict JSON; serialize with an explicit missing-value policy.
- Diagnostics share `subject`/`message`/`capability` fields; `None` means the engine
  supplied no evidence, and no status is inferred from diagnostic prose.
- Column labels expose `source_attribute` (`source` remains an alias). Dataset results
  expose `dataset_path` alongside the `dataset`/`path` keys.

## Metadata, CRS and cross-section bindings

```python
numeric_capability(path, dataset_path)
temporal_reduce(path, dataset_path, offset=None, threshold=None,
                max_block_bytes=16 << 20, threads=None, parallel=True)
raw_crs_evidence(path)
typed_group_attributes(path, group="/")
mesh_inventory(path, max_bytes=67108864)
mesh_name_diagnostics(path, max_bytes=67108864)
cross_sections(path, max_bytes=67108864)
plan_result_mode(path)
steady_profile_names(path, max_bytes=67108864)
read_cross_section_result(path, kind, variable, rows, sections,
                          max_bytes=67108864, threads=None, parallel=True,
                          *, metadata_max_bytes=67108864)
plan_capabilities(path, max_bytes=67108864)
```

`rows`/`sections` are `(start, exclusive_stop, positive_step)`; `kind` is
`steady`/`unsteady`; `variable` is `Water Surface`/`Flow`; `max_bytes` bounds selected
returned bytes. Attribute types and capability status/code/reason are explicit; raw bytes
remain available for unsupported/corrupt attributes. CRS statuses are raw lexical evidence:
capability Supported means lexically readable text only, never a validated semantic CRS or
an EPSG/authority. `exact_wkt` is kept verbatim, including when a lexical capability
refuses it. Hydraulic-unit evidence is separate. Cross-section identities and bank/name
field availability keep engine semantics. Plan capabilities never certify solver success;
`plan_capabilities` includes `structures` (per-family capability, datasets, diagnostics)
and maps `boundary_conditions`/`calibration_regions` alongside reference lines and points.

`mesh_inventory` delegates the canonical engine inventory: physical/stored/ghost counts,
per-variable capability, legacy `results` and `output_blocks` with
`mode`/`block_name`/`dataset_root`/`results`. Names-only layouts fail closed.
`mesh_name_diagnostics` returns `mesh_name` plus typed issue strings `missing_group`,
`extra_group`, `duplicate_attribute_name` and `invalid_name`; inventory disagreement raises
`CorruptDataError`, never a partial successful inventory. Steady absence versus
unsupported layouts and block corruption follow engine capabilities.

`temporal_reduce` uses the engine's `output.chunk_shape` and `output.values`
rows/maximum length, without a second `numeric_capability` query. `chunk_shape` `None`
covers layouts with no chunk dimensions; it does not promise support for every
compact/virtual layout.

## Time and unit bindings

```python
unsteady_time_axis(path, max_bytes=67108864)
unsteady_row_times(path, rows, max_bytes=67108864)
result_units(path, max_bytes=67108864)
dataset_unit_evidence(path, dataset)
```

Time dictionaries keep `raw_strings`, `raw_ms_strings`, `raw_ms`, `days`,
`ms_since_epoch`, `timestamps` (ISO-like `.mmm` strings with no zone), `hour_2400_rows` and
`capability`. Axis reports include `group`, `rows`, `epoch=civil_unix_1970` and
`ms_source`; selections report the original `row_indices` and selected hour-2400 indices
(not renumbered). `timezone` is `None`, never UTC: the epoch is a civil 1970 origin, not a
UTC instant. Integer-ms sources keep a separate `integer_ms` Unsupported capability; no
epoch is invented from raw integers. If valid stamp strings independently prove civil
timestamps, overall capability is Supported while `integer_ms` still reports its
limitation. Days-only axes remain unsupported for civil conversion. Selection validates the
full axis under `max_bytes`, then applies `(start, stop, step)`.

Unit dictionaries include `status`, `text`, `raw_bytes`, `capability`,
`source_attribute`, `quantity_name` and `unit_system_kind` where relevant. `result_units`
returns `variables` (family, area, variable, dataset, units) and `unit_system` evidence
(source group, kind, value). `dataset_unit_evidence` delegates the engine's
`dataset_units`, keeping its Python name. Known quantity labels and exact source text come
from the engine; the binding does not normalize units, infer CRS, resolve unit-system
conflicts or convert numeric values.

## Structure bindings

```python
structure_inventory(path, family, max_bytes=67108864)
read_structure_series(path, family, structure, relative, rows, columns,
                      max_bytes=67108864, threads=None, parallel=True)
```

Family keys: `sa_2d_area_connection` (canonical; `sa2d_area_connection` is accepted as a
compatibility alias), `lateral_structure`, `storage_area`. Rows/columns are
`(start, stop, stride)`. `structure=None` selects only storage-area family series. The
inventory returns the source group, `time_rows`, identities, Node Pointer evidence, series
shape/kind/capability, exact label bytes and units, static dataset paths and label
diagnostics. A successful inventory status certifies the engine's layout validation, not
every column-label table; per-series columns and inventory diagnostics keep
unsupported/corrupt label evidence. No identity, unit or absent series is invented. Reads
keep `f32`/`f64` dtype, shape and bit values, engine path validation, explicit byte
budgets and the standard error mapping. ras-commander's tests use the engine's
`tests/fixtures/structures` fixtures, whose manifest is the hash
oracle.

## Mesh result and geometry bindings

Implemented in `src/python/fim.rs`: mesh summary, series and boundary-condition
inventories and reads, extrema with first stored indices, geometry arrays and packed
families, exact identity comparison, layer inventory and typed run-summary attributes;
plus `mesh_geometry` (field descriptors), `mesh_geometry_tables` (exact Attributes compound
and Cell Info), `geometry_family_capability` and `mesh_boundary_links`. Semantics are the
engine's ([mesh-results.md](mesh-results.md), [engine-api.md](engine-api.md)). Mesh series
descriptors and catalogue mesh-result layers carry `domain_rule` (`stored_location`,
`legacy_exact_name`, `labelled_summary`, `boundary_columns`, `unqualified`).

## 1D bindings

Implemented in `src/python/one_d.rs`: `xs_geometry_inventory`, `read_xs_geometry`,
`one_d_line_family`, `river_reaches`, `river_junctions`, `xs_interpolation_surfaces`,
`xs_result_inventory`, `read_xs_result`, `unsteady_row_window`, `one_d_summary_inventory`,
`read_one_d_summary`, `structure_geometry`, `one_d_geometry_group`, `pump_stations`,
`one_d_structure_inventory`, `one_d_structure_capabilities` (with the engine
`OneDStructureKind::qualified` flag) and `read_one_d_structure_series`. Semantics are in
[one-d.md](one-d.md).

## Read family bindings

Implemented in `src/python/wave3.rs`:

| Binding | Engine API | Returns |
|---|---|---|
| `read_families()` | `READ_FAMILIES` | family names (`pipe_geometry`, `pipe_results`, `reference_line_results`, `reference_point_results`, `breach`, `plan_metadata`, `land_cover`, `infiltration`, `sediment`, `encroachments`, `terrain`, `event_conditions`, `steady_output`, `terrain_modifications`, `mesh_corner_elevations`, ...) |
| `read_family_inventory(path, family, metadata_max_bytes)` | `read_family_inventory` | `{family, capability, datasets, groups}`; each dataset has path, identity, shape, kind/dtype, storage, capability, typed attributes, role, domain, time_group, identity_table, locations, names, geometry_locations, `stored_location_ids` (stored-dtype array or None), time_of_extreme_row |
| `read_family_dataset(path, family, dataset, slices=None, metadata_max_bytes, max_bytes)` | `read_family_dataset` | stored values (below) |
| `output_block_inventory(path, max_bytes)` | `output_block_inventory` | non-Base blocks with stored descriptors |
| `read_output_block(path, block, relative, slices=None, max_bytes)` | `read_output_block` | stored values |
| `reference_point_geometry(path, max_bytes)` | `reference_point_geometry` | `Reported`: Attributes compound, Points array, features (identity, name, area, stored i32 cell_index, point_row) |
| `classification_catalogue(path, family, max_bytes)` | `classification_catalogue` | `Reported`: layout, entries (pixel_id as stored int, name, variable_rows, capability), unmatched_variables |
| `mesh_corner_elevations(path, mesh, layout, expected_corners=None, max_bytes)` | `mesh_corner_elevations` | `Reported`: mesh, layout (`geometry` or `mapping_cache`), dataset descriptor (path, identity, shape, attributes), corner_count, corner_count_source (`{kind: coordinates, path}` or `{kind: caller}`), values (`float32` ndarray as stored). Additive within API 11 (engine 0.2.3); check with `hasattr`. |
| `stored_family_index(path, metadata_max_bytes)` | all family inventories + output blocks | compact routing index: `{datasets: [{owner, path, storage, shape, dtype, status, code}], failures, output_blocks}` |

Stored values are `{"storage": "numeric", "values": ndarray}` (stored dtype),
`{"storage": "compound", "values": {shape, raw_bytes, columns}}`,
`{"storage": "bitfield_bytes", "shape", "raw_bytes"}` (exact bytes; h5py reads these
1-byte bitfields as `uint8`) or `{"storage": "strings", "shape", "values"}` (engine-decoded
text, trailing NULs removed). Slices are `(start, stop, step)` per dimension and apply to
numeric storage only. Family-level errors in `stored_family_index` are reported in
`failures`, never dropped; the ras-commander router also surfaces them in required-mode
refusals and in its trace.

## Typed attribute and raw string bindings

| Binding | Engine API | Returns |
|---|---|---|
| `typed_dataset_attributes(path, dataset, max_bytes)` | `typed_dataset_attributes` | `{capability, value}`; value is a list of typed attributes (as `typed_group_attributes`) for any dataset, inside or outside the read families. Absent dataset: Unavailable; group path: `invalid_request` |
| `raw_fixed_strings(path, dataset, max_bytes)` | `raw_fixed_strings` | `{capability, value: {shape, dataspace, width, padding, charset, raw_bytes}}`: stored bytes with padding kept (h5py's bytes follow after the NULLPAD conversion) |

Every typed attribute carries `dataspace` (`scalar`, `simple` or `null`; `null` is h5py
`Empty`), and bitfields are `attribute_type = {kind: "bitfield", bytes, byte_order}` with
the exact stored bytes as `value`. Compound tables and stored family datasets carry
`dataspace`.

## How ras-commander routes reads

These behaviours live in ras-commander's Python adapter, not in this module, but they
define how the bindings are used:

- `fast_hdf.read_dataset` dispatches bounded Base Output summary, series and BC reads to
  the typed readers under the backend policy. Explicit thread controls and reads larger
  than the typed budget keep the generic native reader. Metadata, geometry construction,
  ghost filtering, timestamps and public exceptions stay in the Python adapters; h5py
  remains the automatic unsupported fallback. HEC-RAS 7.0 `Cell Hydraulic Depth` is passed
  through exactly (no `Depth` alias); the xarray name normalizer only lowercases and
  replaces punctuation and spaces (`cell_hydraulic_depth`).
- `fast_hdf.read_typed` (`ras_commander/_fast_hdf_typed.py`) uses `stored_family_index` as
  the membership oracle for datasets outside the mesh-geometry and 1D routes: supported
  numeric, compound and bitfield datasets are read typed; listed-but-unqualified datasets
  (for example Davis pipe summaries with no stored `Location`) keep their exact literal
  values through the generic reader; strings stay on h5py (the documented metadata
  exception), because decoded text cannot prove h5py's fixed-length bytes. Compound members
  stored NULLTERM or SPACEPAD are converted the way HDF5 converts them into h5py's NULLPAD
  memory type (cut at the first NUL / strip trailing spaces, zero-fill), from the engine's
  member padding evidence.
- The Python temporal fallback supplies all timing and read-plan keys, with NaN for
  unavailable setup/splits and a `timing_scope` explaining the combined measurement. Native
  setup is measured; both backends expose `engine_seconds`. `parallel` is forwarded to the
  native session and reduction; the Python fallback is serial. Fallback
  `read_plan.chunk_cache_bytes` is the h5py/HDF5 configured raw-data chunk cache capacity;
  native reports the engine file cache's maximum capacity. These are distinct backend
  configurations, not measured bytes or process RSS, and their equality is not a parity
  promise; block plan/count/value and peak/count arrays are tested for parity.
- `log_call` logs function names only and never formats array arguments or results.

## Dependency provenance

* `hdf5-reader`/`hdf5-core` are vendored in this repository's `vendor/` directory and
  compiled into the crate as private modules, based on upstream `roteiro-gis/netcdf-rust` tag `v0.9.3`
  (`3d708fd40c6809f5d215bd2d9f15e9a05de84bd0`) plus 14 local patches, `MIT OR Apache-2.0`.
  `vendor/hdf5-reader/PROVENANCE.md` and `vendor/LOCAL_PATCHES.md` record the local
  changes. The extension has no direct generic-reader dependency.
* `freestiler-core`: `walkerke/freestiler`, revision
  `beb64b0d4414da895ec67d02b2f91db6ab6acc37`, MIT AND ISC. Its crate is unpublished, so its
  source is embedded from `vendor/freestiler-core` (feature `python`; see
  `vendor/freestiler-core/PROVENANCE.md`). `vendor/PROVENANCE.json` records both upstreams
  and the file hashes.
* PyO3/rust-numpy provide the binding. ndarray allocations transfer to NumPy on numeric
  return. Owned input copies are intentional and are not zero-copy.
* `flate2` enables zlib-rs. Cargo feature unification also enables upstream
  `rust_backend`; flate2 selects the explicitly requested zlib-rs backend.
* `i_overlay` 1.9.4 and `i_float` 1.6.0 (already in freestiler's dependency tree) supply
  integer-grid repair; their licence notices are included. `prost` uses the same 0.13
  version as freestiler, preserving MVT field representations.

`Cargo.lock` records exact transitive dependencies. Retain upstream licence notices
when distributing wheels. Source provenance of the extension and engine is in
[provenance.md](provenance.md).

### Licences

The wheel ships these files under `.dist-info/licenses/`, through maturin
`license-files` in `pyproject.toml`:

- `licenses/ras-commander-fast-hdf-LICENSE.txt`: a copy of this repository's MIT `LICENSE`
  (CLB Engineering Corporation). Keep it identical to `../LICENSE`.
- `licenses/ras-commander-LICENSE.txt`: the original ras-commander MIT notice (William M.
  Katzenmeyer), for the code extracted from ras-commander.
- the other `licenses/*.txt` per-crate notices.
- `THIRD-PARTY-NOTICES.md`: generated from the locked release-wheel graph
  (`python,extension-module,exports-all,exports-pmtiles`). It lists the engine (as
  `ras-commander-fast-hdf`, the name ras-commander's wheel gate looks for), the
  embedded vendor sources and every crate linked for any target platform, with the full
  licence texts each crate ships. It includes the bundled zstd C library (BSD-3-Clause,
  elected from BSD-3-Clause OR GPL-2.0) and the Apache-2.0-only `approx` and `prost`.
  Compile-time proc-macro crates (such as `prost-derive`) are listed too, marked as such.

Regenerate the notices after any change to `Cargo.lock`. From the repository root:

```text
python scripts/generate_third_party_notices.py --features python,extension-module,exports-all,exports-pmtiles --title "ras-commander-fast-hdf wheel (release features)" --output python/licenses/THIRD-PARTY-NOTICES.md
python scripts/generate_third_party_notices.py --features python,extension-module,exports-all,exports-pmtiles --title "ras-commander-fast-hdf wheel (release features)" --output python/licenses/THIRD-PARTY-NOTICES.md --check
```

The script uses only the Python standard library and `cargo metadata --locked`; the same
script writes the crate's `THIRD-PARTY-NOTICES.md` (see [../building.md](../building.md)). A
crate whose published package ships no licence file needs its upstream texts under
`scripts/licence-fallbacks/<crate>-<version>/`, with a `SOURCE.txt`, and generation fails
until they are added. A wheel built with other features links a different graph:
regenerate with those features before distributing it.
Tested datasets, measured timings and limitations are in [validation.md](validation.md).

## API version history

| API | Engine API | Added or changed |
|---|---|---|
| 5 | 5 | Hardened bindings: `numeric_capability`, `temporal_reduce`, `raw_crs_evidence`, `typed_group_attributes`, `mesh_inventory`, `cross_sections`, `plan_result_mode`, `steady_profile_names`, `read_cross_section_result`, `plan_capabilities`; central typed exception mapping; time/unit bindings and `mesh_name_diagnostics`; structure bindings; unified wire shapes (capability `{status, code, reason}`, `Reported<T>` evidence, `{source, table, row}` identities, snake_case enum strings, optional `max_bytes` defaults, dictionary diagnostics). |
| 6 | 5 | Mesh result and geometry bindings in `src/python/fim.rs` (45 entry points). |
| 7, 8 | 6 | Sessions (`HdfSession`, NumPy element/index outputs), cache controls (`cache_bytes`, `cache_slots`, `cache_stats()`), `read_mesh_summary_values` and `read_mesh_boundary_batch`; see [sessions.md](sessions.md). |
| 9 | 7 | 1D bindings (`src/python/one_d.rs`) and the mesh-geometry tables, geometry family capability and boundary links (67 entry points). |
| 10 | 8 | Read family bindings (`src/python/wave3.rs`) and `stored_family_index` (75 entry points). |
| 11 | 9, then 10 | `typed_dataset_attributes`, `raw_fixed_strings`, `dataspace` and bitfield attribute types, `domain_rule`; panic containment and `_panic_for_test` (78 entry points). Built against engine API 10: ring-order contract, malformed-input hardening and the 2 GiB default read budget; bindings unchanged. Additive within API 11 (engine 0.2.3): `mesh_corner_elevations` (79 entry points), `SESSION_SOURCES` (sessions as typed-read sources) and `HdfSession.read_mesh_face_lines`. |
