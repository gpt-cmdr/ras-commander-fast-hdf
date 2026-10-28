# Arrow layers (canonical layer catalog)

The engine returns HEC-RAS layers as Arrow record batches: the 2D mesh cells, faces and area
perimeters, and every stored 2D summary variable. The data go from the HDF file to Arrow
memory without Python objects. Writing files (GeoParquet, PMTiles, FlatGeobuf, ...) is not
part of this API. terrio owns file formats; see [Format writers](#format-writers).

| Feature | Gives | Adds |
|---|---|---|
| `arrow` | `arrow_layers` module (Rust) | `hdf`, `mesh`, `results` plus `arrow-array`, `arrow-schema`, `arrow-buffer` |
| `python-arrow` | `arrow_layer()`, `layer_catalog()` in the Python extension | `python` + `arrow` + Arrow's C data interface (no new crates) |

`hdf`, `mesh` and `results` are unchanged: a build without `arrow` has no Arrow dependency.

## Catalog

The engine owns the layer names, geometry types, identities and field types. Consumers keep
their own names through aliases, so no existing archive is renamed.

| Canonical layer | Geometry | Identity | Aliases | Status |
|---|---|---|---|---|
| `mesh_cells` | Polygon | `mesh_name`, `cell_id` | | Arrow |
| `mesh_faces` | LineString | `mesh_name`, `face_id` | | Arrow |
| `mesh_perimeters` | Polygon | `mesh_name` | ras2cng `mesh_areas` | Arrow |
| `<variable slug>` (2D summary) | none (join by identity) | `mesh_name`, `cell_id` or `face_id` | | Arrow |
| `mesh_cell_points` | Point | `mesh_name`, `cell_id` | | planned |
| `river_centerlines`, `river_bank_lines`, `river_edge_lines` | LineString | `source_row` | ras2cng `centerlines`, `bank_lines`, `edge_lines` | planned |
| `cross_sections`, `river_flow_paths`, `structures`, `bc_lines`, `breaklines`, `reference_lines` | LineString | `source_row` | | planned |
| `storage_areas`, `refinement_regions`, `mannings_n_regions` | Polygon | `source_row` | | planned |
| `reference_points`, `river_junctions`, `pump_stations` | Point | `source_row` | | planned |

The names are the engine family names that terrio's HECRAS driver already uses. "Planned"
layers are read today through the typed family APIs ([Read families](read-families.md)); they
become Arrow layers one by one, each with the same equivalence proof.

## Fields

- `mesh_name`: Utf8, the `Geometry/2D Flow Areas/Attributes` `Name`.
- `cell_id`, `face_id`: Int64, the stored zero-based index within the mesh.
- `geometry`: Binary, little-endian 2D OGC WKB, field metadata
  `ARROW:extension:name = geoarrow.wkb`. The engine attaches no CRS. The caller resolves it
  from the stored `Projection` evidence; no EPSG code is guessed.
- Summary layers, named by the variable's slug (`Maximum Water Surface` becomes
  `maximum_water_surface`):
  - `<slug>`: the stored type (Float32 for HEC-RAS floats; integers become Int64, or UInt64
    when stored unsigned). NaN is null.
  - `<slug>_time`: present when the engine proves row 1 of a two-row summary is `Time`.
    It is the simulation start (`Plan Information` `Simulation Start Time`, strictly
    `ddMMMyyyy HH:MM:SS`) plus the stored days, computed as pandas does and rounded half to
    even to 100 ms. The type is Timestamp(us) when every stored offset is a whole number
    of days (what pandas 3 infers), Timestamp(ns) otherwise.
  - `is_ghost_cell`: Bool, cell layers only. Ghost cells (index at or above `Cell Count`)
    are left out, so the value is always false.

Rows follow the meshes in Attributes order, then the stored order. Mesh cells are the
engine's assembled rings (see [Engine API](engine-api.md)), closed. Cells with fewer than
three faces are left out.

## Refusals

Each layer equals what ras-commander's Python readers give, or it is refused with a typed
error. The engine refuses, rather than approximates, wherever ras-commander would leave its
native path or read something the engine cannot prove:

- A mesh without the six topology arrays, a cell quarantined as invalid geometry, or an
  assembled cell with fewer than three faces. In each case ras-commander uses its Python
  polygonizer instead.
- A partial set of face tables, or an area without a `Perimeter` dataset.
- An Attributes table whose first column is not `Name`; empty, duplicate or non-component
  mesh names; names ras-commander converts to booleans, dates or durations.
- Summary datasets the engine does not qualify, such as legacy layouts without a stored
  `Location`, or shapes other than `(n)` and `(2, n)`. For a `(2, n)` dataset, row 1 must be
  proven to be `Time`.
- A start time in any other form, a 24:00 start, offsets outside the timestamp range, and
  meshes whose time offsets differ in resolution.
- A variable that no mesh stores gives `NotFound`, not an empty layer.

## Python

```python
import pyarrow as pa
import ras_commander_fast_hdf as fh

cells = pa.table(fh.arrow_layer("Model.g01.hdf", "mesh_cells"))
areas = pa.table(fh.arrow_layer("Model.g01.hdf", "mesh_areas"))     # alias of mesh_perimeters
wse = pa.table(fh.arrow_layer("Model.p01.hdf", "mesh_summary", "Maximum Water Surface"))
polygons = cells.join(wse, ["mesh_name", "cell_id"], join_type="left outer")
```

`arrow_layer` returns an `ArrowLayer` that implements the Arrow PyCapsule interface
(`__arrow_c_stream__`, `__arrow_c_schema__`). Any PyCapsule consumer can read it without
IPC files or copies through Python: pyarrow, polars, DuckDB, and GeoPandas through pyarrow.
The stream can be consumed once. `layer_catalog()` returns the catalog above as JSON.

## Equivalence

`scripts/arrow_layers_equivalence.py` compares every layer with ras2cng's Python pipeline
(ras-commander 0.110.1 with the released 0.2.3 wheel as the reference). It checks column
names, Arrow types, rows, row order, values and WKB, using the alias `mesh_areas`. It also
rebuilds ras2cng's polygon-mode result layer with a plain pyarrow join and compares it.

Results, 2026-10-28:

- 20 files: the 16 Bald Eagle p07 project files (15 geometry, p07 plan), Bald Eagle bridges
  (g03, p05) and Davis (g02, p02). Every `mesh_cells`, `mesh_faces` and `mesh_areas` layer
  was equal. Every summary variable ras-commander reads was equal, both as values and as
  the ras2cng polygon layer.
- Spring Creek (g01, p01; 340,163 cells, 761,222 faces): mesh layers equal; WSE summaries
  equal (including a whole-day time column, which pandas types as microseconds). Face
  summaries in this older layout have no stored `Location` and are refused.
- `Cell Last Iteration` (no `Location` attribute) is refused. ras-commander reads it, so
  ras2cng keeps reading it through Python.
- `Starting Differences *` and `Cell Cumulative Iter Lookup` are refused by the engine and
  skipped by ras-commander.

## Format writers

The `exports-*` writers (GeoParquet archive profile, PMTiles bridge, FlatGeobuf, GeoPackage,
GeoJSON) are frozen: bug fixes only, no new options. They retire once terrio's HECRAS driver
(a thin adapter over these layers) is upstream and terrio's GeoParquet writer has the
options ras2cng's archive profile needs:

- Hilbert order within each layer, exactly DuckDB's `ST_Hilbert`, with an optional
  `hilbert_index` column.
- ZSTD with a level.
- A `layer` column, with row groups that end at layer boundaries and hold up to 65,536 rows.
- A GeoParquet 1.1 `bbox` struct covering and a file-level `bbox`.
- Dictionary encoding on Utf8 columns only.
- Chunk-level statistics without a page index.
- A union-of-layers schema with non-nullable identity fields.
