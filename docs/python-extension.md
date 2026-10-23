# Python extension: `ras_commander_fast_hdf`

The wheel `ras-commander-fast-hdf` installs one compiled module,
`ras_commander_fast_hdf`. It is an internal building block for ras-commander:
applications should call ras-commander's `Hdf*` classes, which choose between
this module and h5py and build the pandas, xarray and GeoPandas results. The
functions below are documented so that the contract between the two packages
is reviewable, not as a stable end-user API.

The complete, generation-by-generation contract is in
[reference/extension-contract.md](reference/extension-contract.md).

## Versions and build identity

| Name | Meaning |
|---|---|
| `__version__` | Package version, e.g. `"0.1.0"`. |
| `API_VERSION` | Binding contract version (11). ras-commander requires an exact match. |
| `TEXT_API_VERSION` | Separate version of the RAS text-file bindings (1). |
| `ENGINE_COMMIT` | Commit of this repository the wheel was built from, or `"unknown"` for builds without Git metadata. |
| `ENGINE_FEATURES` | Engine Cargo features compiled in: `("catalog", "hdf", "mesh", "results")`. |

```python
import ras_commander_fast_hdf as native
native.__version__, native.API_VERSION, native.ENGINE_COMMIT
```

## Errors

Engine errors map to Python exceptions by kind:

| Engine error kind | Python exception |
|---|---|
| `unsupported` (unproven layout, datatype, encoding, or over budget) | `UnsupportedError` (`ValueError`) |
| `corrupt` (damaged or self-contradictory data) | `CorruptDataError` (`ValueError`) |
| `structural_topology`, `invalid_geometry` | `TopologyError` (`ValueError`) |
| `invalid_request` (bad selection, budget or argument) | `ValueError` |
| `not_found` (missing HDF object or attribute) | `KeyError` |
| `file_not_found` | `FileNotFoundError` |
| `io` | `OSError` |
| `internal` | `RuntimeError` |

The RAS text bindings raise `TextParseError`, `TextNotFoundError` and
`TextEditError` (all `ValueError`). A contained Rust panic raises
`NativePanicError` (`RuntimeError`); under ras-commander it is re-raised as
`fast_hdf.FastHdfPanicError`. ras-commander treats every one of these as a
refusal: `auto` falls back to h5py, `native` raises `FastHdfError`.

## Sessions

```python
with native.HdfSession(path, threads=None, parallel=True,
                       cache_bytes=None, cache_slots=None) as s:
    a = s.read_numeric("Results/.../Depth", selection=[(0, 10, 1), (0, 500, 1)])
```

An `HdfSession` keeps one read-only file open with its metadata and chunk
caches. Closing it is idempotent, waits for reads in progress, and releases the
file. Arrays returned before closing stay valid. ras-commander opens one
session per public call and closes it when the call returns; see
[reference/sessions.md](reference/sessions.md).

Session methods include `read_numeric`, `read_numeric_batch`, mesh summary,
series and boundary-condition reads (`read_mesh_summary`,
`read_mesh_summary_values`, `read_mesh_series`, `read_mesh_boundary_series`,
`read_mesh_boundary_batch`), their inventories and `reduce_mesh_series`.
Sizes are bounded by `max_bytes` and `metadata_max_bytes` (64 MiB default per
call for these typed reads).

## Module-level reads

Each opens the file for one call. The groups below list every function; the
reference gives arguments and return shapes.

| Group | Functions |
|---|---|
| Numeric datasets | `read_numeric(path, dataset_path, selection=None, threads=None, parallel=True)`, `read_numeric_batch(path, dataset_paths, ...)`, `numeric_capability` |
| Attributes and text | `typed_group_attributes`, `typed_dataset_attributes`, `raw_fixed_strings`, `raw_crs_evidence`, `dataset_unit_evidence`, `result_units` |
| Capabilities and inventories | `capabilities`, `plan_capabilities`, `plan_result_mode`, `layer_inventory`, `stored_family_index`, `output_block_inventory`, `read_output_block`, `run_summary` |
| Stored table families | `read_families`, `read_family_inventory`, `read_family_dataset`, `classification_catalogue`, `geometry_family`, `geometry_family_capability`, `structure_geometry`, `structure_inventory`, `reference_point_geometry`, `pump_stations` |
| 2D mesh geometry | `mesh_inventory`, `mesh_geometry`, `mesh_geometry_tables`, `read_mesh_geometry`, `mesh_name_diagnostics`, `compare_mesh_identity`, `validate_geometry` |
| 2D mesh results | `mesh_summary_inventory`, `mesh_series_inventory`, `mesh_boundary_inventory`, `mesh_boundary_links`, `read_mesh_summary`, `read_mesh_series`, `read_mesh_boundary_series`, `reduce_mesh_series`, `temporal_reduce` |
| 1D geometry and results | `cross_sections`, `xs_geometry_inventory`, `read_xs_geometry`, `xs_interpolation_surfaces`, `river_reaches`, `river_junctions`, `one_d_geometry_group`, `one_d_line_family`, `xs_result_inventory`, `read_xs_result`, `read_cross_section_result`, `steady_profile_names` |
| 1D summary and structures | `one_d_summary_inventory`, `read_one_d_summary`, `one_d_structure_inventory`, `one_d_structure_capabilities`, `read_one_d_structure_series`, `read_structure_series` |
| Time | `unsteady_time_axis`, `unsteady_row_times`, `unsteady_row_window` |

## Kernels used by ras-commander

| Function / class | Purpose |
|---|---|
| `assemble_cells(face_points, face_endpoints, face_perimeter_info, face_perimeter_values, cell_face_info, cell_face_values, threads=None, *, strict=True, return_diagnostics=False)` | Builds 2D cell polygons from the stored face topology: packed coordinates, ring offsets and cell IDs, in the same ring and vertex order as ras-commander's Python polygonizer. |
| `assemble_rings`, `face_graph`, `NativeAnalysis` | Fluvial/pluvial boundary and polygon kernels (`HdfFluvialPluvial`). |
| `write_pmtiles(layers, output, min_zoom, max_zoom, threads=None)` | Writes mesh polygons to a PMTiles vector tile archive. |
| `export_ipc_file`, `export_pmtiles_ipc_file`, `validate_export_wkb` | Vector exports (GeoJSON, FlatGeobuf, GeoParquet, GeoPackage, PMTiles) from Arrow IPC; present when built with the `exports-*` features. |
| `text_index`, `text_geometry_index`, `text_geometry_edit`, `text_apply_edits` | Fast parsing and editing of HEC-RAS text files (`ras_commander.fast_text`). |

## Threads and the GIL

Reads release the GIL while the engine works. `threads=None` uses the number
of available CPUs, at most 8; `threads=n` (1 to 1024) uses `n`. Thread pools are
shared per size. `parallel=False` reads serially. Input NumPy arrays are copied before the GIL
is released, so concurrent mutation by Python cannot affect a running kernel.
