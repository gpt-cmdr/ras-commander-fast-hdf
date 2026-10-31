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

| Canonical layer | Geometry | Identity | ras2cng name | Source |
|---|---|---|---|---|
| `mesh_cells` | Polygon | `mesh_name`, `cell_id` | same | assembled cell rings |
| `mesh_cell_points` | Point | `mesh_name`, `cell_id` | (results point mode) | `Cells Center Coordinate`, physical cells |
| `mesh_faces` | LineString | `mesh_name`, `face_id` | same | face lines with perimeter points |
| `mesh_perimeters` | Polygon | `mesh_name` | `mesh_areas` | `Perimeter` |
| `<variable slug>` (2D summary) | none, or the cell/face geometry on request | `mesh_name`, `cell_id` or `face_id` | same | `Summary Output/2D Flow Areas` |
| `bc_lines` | LineString | `source_row` | same | `Geometry/Boundary Condition Lines` |
| `breaklines` | LineString | `source_row` | same | `Geometry/2D Flow Area Break Lines` |
| `refinement_regions` | Polygon | `source_row` | same | `Geometry/2D Flow Area Refinement Regions` |
| `reference_lines` | LineString | `source_row` | same | `Geometry/Reference Lines` |
| `reference_points` | Point | `source_row` | same | `Geometry/Reference Points` |
| `mannings_n_regions` | Polygon | `source_row` | same | `Geometry/Land Cover (Manning's n)` |
| `cross_sections` | LineString | `source_row` | same | `Geometry/Cross Sections` (cut lines) |
| `river_centerlines` | LineString | `source_row` | `centerlines`, `river_reaches` | `Geometry/River Centerlines` |
| `river_bank_lines`, `river_edge_lines` | LineString | `source_row` | `bank_lines`, `edge_lines` | `Geometry/River Bank Lines`, `.../River Edge Lines` |
| `river_flow_paths` | LineString | `source_row` | | `Geometry/River Flow Paths` |
| `structures` | LineString | `source_row` | same | `Geometry/Structures` centerlines |
| `storage_areas` | Polygon | `source_row` | same | `Geometry/Storage Areas` |
| `river_junctions` | Point | `source_row` | | `Geometry/Junctions` |
| `pump_stations` | Point | `source_row` | same | `Geometry/Pump Stations` |
| `pipe_conduits` | LineString | `source_row` | same | `Geometry/Pipe Conduits` |
| `pipe_nodes` | Point | `source_row` | same | `Geometry/Pipe Nodes` |

The names are the engine family names that terrio's HECRAS driver already uses. Every layer is
read as Arrow. `read_layer(name, variable, options)` (Rust) and `arrow_layer(path, name, ...)`
(Python) take the canonical name or any alias. A family the file does not store is `NotFound`.

### Field aliases

Consumers keep their own column names through `FIELD_ALIASES` (Rust) or the `field_aliases`
list of `layer_catalog()` (Python). Each entry is `(consumer, consumer layer, canonical field,
alias)`; the values are unchanged.

| Consumer | Layer | Canonical | Alias |
|---|---|---|---|
| ras2cng | `bc_lines` | `source_row` | `bc_line_id` |
| ras2cng | `breaklines` | `source_row`, `Cell Spacing Near`, `Cell Spacing Far`, `Near Repeats`, `Protection Radius` | `bl_id`, `cell_spacing_near`, `cell_spacing_far`, `near_repeats`, `protection_radius` |
| ras2cng | `refinement_regions` | `source_row` | `rr_id` |
| ras2cng | `mannings_n_regions` | `source_row` | `region_id` |
| ras2cng | `storage_areas` | `Mode`, `Avg Area`, `Min Elev` | `mode`, `avg_area`, `min_elev` |
| ras2cng | `river_reaches` | `source_row` | `river_id` |
| terrio | `mesh_summary` | `is_ghost_cell` | `is_ghost` |

Columns a consumer derives or joins are not aliases and are not in the canonical layer. For
ras2cng these are: `length` (river lines), `station_elevation`, `mannings_n`, `n_lob`/
`n_channel`/`n_rob`, `ineffective_blocks` and the levee columns (cross sections), the joined
structure tables (bridge coefficients, gates, `Profile_Data`, ...), and the all-null union
columns its result layers carry. A consumer that needs them derives them from the canonical
layer or keeps its Python path.

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

- Summary layers with `summary_geometry` (Python `geometry=True`): a last `geometry` field,
  the cell polygon (`mesh_cells`) or face line (`mesh_faces`) read from the same file, null
  where the element has no geometry row. ras2cng's polygon-mode result layer is these rows
  and values; it adds the all-null columns of its geometry union, which the canonical layer
  does not carry.

Stored feature layers (every non-mesh layer):

- `source_row`: Int64, the zero-based row of the stored `Attributes` table (of `Info` when
  the family has no `Attributes`). This is the row identity ras-commander numbers as
  `bc_line_id`, `bl_id`, `rr_id`, `region_id` or `river_id`.
- Every stored `Attributes` column under its stored name, in stored order: text as Utf8
  (decoded as h5py does), signed integers as Int64, unsigned as UInt64, floats in their stored
  width with NaN as null. Bitfield and unproven columns are refused. Text is never converted
  (ras-commander turns some strings into booleans or dates).
- `geometry`: one part is a LineString or Polygon; several line parts are a MultiLineString,
  as ras-commander builds them. A polygon ring is closed when stored open (as Shapely closes
  it). A feature with no points (no part, or only empty parts) has a null geometry, as
  ras-commander gives for a structure without a drawn centerline. Multi-part polygons are
  refused: ras-commander assembles them per family (holes by containment, or a MultiPolygon).
  Points stored with more than two ordinates are refused.

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
# or, with the cell polygons read from the plan file itself:
wse_polygons = pa.table(fh.arrow_layer("Model.p01.hdf", "mesh_summary", "Maximum Water Surface", geometry=True))
bc = pa.table(fh.arrow_layer("Model.g01.hdf", "bc_lines"))
```

`arrow_layer` returns an `ArrowLayer` that implements the Arrow PyCapsule interface
(`__arrow_c_stream__`, `__arrow_c_schema__`). Any PyCapsule consumer can read it without
IPC files or copies through Python: pyarrow, polars, DuckDB, and GeoPandas through pyarrow.
The stream can be consumed once. `layer_catalog()` returns the catalog above as JSON (`layers` and `field_aliases`).

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

### Feature layers and summary geometry (2026-10-29)

`scripts/arrow_feature_layers_equivalence.py` compares every non-mesh layer with what
ras2cng's `_extract_hdf_layer` returns (ras2cng 0.12.0, ras-commander 0.110.3, released
0.2.4 wheel as the reference), after applying the ras2cng field aliases: row count, row order
through the identity, WKB of every geometry, and the value of every shared column (the
canonical column cast to ras2cng's type).

- 17 geometry files (the 15 Bald Eagle p07 project files, Bald Eagle bridges g03, Davis g02):
  240 comparisons, 0 different. Layers seen: `mesh_cell_points`, `bc_lines`, `breaklines`,
  `structures`, `storage_areas`, `mannings_n_regions`, `cross_sections`, `centerlines`,
  `river_reaches`, `pump_stations`.
- Type differences (values equal): ras-commander widens float32 to float64 for the breakline
  spacings, storage-area `avg_area`/`min_elev` and centerline junction distances; it stores
  `Protection Radius` as int64 where the canonical layer keeps the stored unsigned type.
- No corpus file stores refinement regions, reference lines or points, river flow paths,
  bank or edge lines, junctions or pipe networks; those layers are covered by the synthetic
  fixture and the shared rules only.
- Summary layers with `geometry=True` (cells and faces read from the plan file) equal
  ras2cng's polygon-mode result layers (cells from the geometry file) on Bald Eagle p07,
  bridges p05 and Davis p02: 18 variables, 0 different.

### Time and memory, Bald Eagle p07 (2026-10-29)

Fresh process per step, best of three, pyarrow and pandas imported in both, shared host.

| Step | Rows | ras2cng/ras-commander -> Arrow | Engine Arrow | Peak working set |
|---|---:|---:|---:|---:|
| g01 every HDF geometry layer (`merge_all_layers`; engine adds `mesh_cell_points`) | 264,350 / 351,389 | 1.64 s | 0.13 s | 891 -> 136 MB |
| p07 six summary variables with cell/face geometry (`merge_all_variables`, polygon mode) | 702,758 | 2.58 s | 0.96 s (geometry per variable) / 0.21 s (values, geometry read once) | 586 -> 136-143 MB |

`geometry=True` re-reads the mesh geometry for every variable; a caller that reads several
variables reads `mesh_cells` and `mesh_faces` once and joins by identity.

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
