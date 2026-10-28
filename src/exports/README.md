# Typed source exports

This is the `exports` module of `ras-commander-fast-hdf` (formerly the separate
crate `ras-export`), compiled only with the `exports-*` features and reached
through the Python extension. It is not part of the crate's public Rust API. It
reads no HDF files, runs no model and performs no coordinate transformation. Its
entry point accepts a schema and an iterator of `Result<RecordBatch>`. A Python
adapter (such as a `fast_hdf.export_arrow` helper in ras-commander) can stream an
Arrow table/reader to temporary IPC and call the native extension. There is no
required production executable. The test harness is only a qualification transport.

## Contract v1

Every schema requires `mesh_name: Utf8` and `cell_id: Int64`, both nonnullable.
The stable identity is their ordered pair, never local cell ID alone. Region,
cross-section and structure producers supply their own layer/mesh namespace and
stable local identifier. Negative local IDs reject. Uniqueness within a batch
is enforced. **Global uniqueness across batches is a caller precondition**;
writers retain duplicates across batch boundaries instead of maintaining an
unbounded in-memory identity registry. No global PMTiles integer ID is invented.

`geometry: Binary` contains nullable XY OGC WKB with extension `geoarrow.wkb`.
Geometry metadata must declare `crs_type: projjson` and the resolved PROJJSON.
Point, LineString, Polygon, MultiPoint, MultiLineString and MultiPolygon support
both byte orders. Polygons preserve holes and multipart membership. Z/M, EWKB,
embedded SRIDs, geometry collections, empty points (NaN coordinates), malformed
counts, truncated/trailing WKB, short/unclosed rings and nonfinite coordinates
reject. Empty lines/polygons/multi geometries and null geometry are supported
except indexed FlatGeobuf, which rejects them explicitly. There is no topology
repair: producers are responsible for full valid polygon topology.

Attributes are nullable Int64, UInt64, Float64, Boolean, Utf8, Binary and timezone
free Timestamp(Millisecond). Time means UTC Unix milliseconds; dates outside a
container's supported calendar reject there. Timezone normalization is a producer
operation. NaN/infinity reject; supply Arrow null explicitly. Nested/dictionary
arrays, narrower numeric widths and LargeUtf8/LargeBinary reject. Convert them
explicitly before export. Case-insensitive duplicate, empty/NUL and reserved
`fid`/`_ras_fid` field names reject. Arrow schemas must remain exactly equal,
including metadata, and each input batch must satisfy `max_batch_rows`.

Schema metadata `ras:export:v1` is JSON with WKT, PROJJSON, units, `axis_order:xy`,
`resolved:true`, and `geographic_wgs84`. Semantic CRS resolution/conflict checks
belong to the producer (pyproj in the Python adapter). Rust checks evidence is
present and consistent with the GeoArrow metadata; it does not parse or compare
WKT using a second CRS engine. A `resolved` flag is a producer assertion, not
independent evidence that a CRS was detected correctly. WKT without an EPSG
match is supported. Buffer coordinate order is always x/y, regardless of
authority axis ordering. Metres, US survey feet and international feet remain
native in binary outputs. No unit-based coordinate conversion is guessed.

## Format profile

| Writer | Geometry and CRS | Attribute conversion and limits |
| --- | --- | --- |
| Arrow IPC file | Original WKB, schema and metadata | Exact supported Arrow types, nulls and full uint64 |
| GeoParquet | WKB GeoParquet metadata, native PROJJSON, bbox/type finalization | Exact supported types, Snappy, 65536-row groups |
| GeoParquet archive profile | As GeoParquet, plus optional bbox covering (GeoParquet 1.1 `bbox` struct, or four `bbox_*` columns) and file-level `bbox` | Archive attribute types (below) passed through unchanged; one or more layers unified into one schema with a `layer` column; Snappy, ZSTD (level) or none; row groups end at layer boundaries unless `layer_row_groups` is false; optional Hilbert order within each layer and Hilbert key column |
| FlatGeobuf | Official writer, native WKT, spatial index, mixed geometries; empty/null rejects | uint64 converts to checked signed64; larger values reject because GDAL otherwise rounds ULong to float64. Time stored as integer milliseconds, schema metadata carries contract. Null properties omitted in feature storage and reopen as null |
| GeoPackage | Complete SQLite container, feature table, SRS/WKT2 extension, RTree and contents tables | SQLite signed64 (larger uint64 rejects), bool, real, UTF8, blob, nullable fields; timestamp stored as UTC ISO text with millisecond precision. Field metadata beyond CRS is not archived |
| GeoJSON / Lines | WGS84 longitude/latitude only, CCW exterior/CW holes; Python transforms and orients | Nullable scalar properties; integers outside ±(2**53-1) become decimal strings. Millisecond timestamps remain JSON integers. Binary attributes reject. No full Arrow schema/field metadata archive. Lines is one feature per newline, with no RFC8142 record separator |

GeoJSON timestamp integers outside JavaScript's exact integer range require a
consumer with an exact integer parser; year3000 timestamps are inside that range.
GeoJSON has no fidelity claim for the original Arrow type schema. Binary source
exports retain geometry coordinates, without rendering simplification or tiling.
PMTiles/freestiler remains the separate rendering path and owner.

## GeoParquet archive profile

`exports::geoparquet::export_layers` (Python: `export_geoparquet_ipc_files(inputs,
output, options)`, `inputs` a list of `(layer_name, ipc_path)`, `options` JSON) writes
one GeoParquet from several layers, as ras2cng's archive does (`Model.g01.parquet` with a
`layer` column). Options, all optional and unknown keys refused:

| Option | Default | Meaning |
| --- | --- | --- |
| `compression` | `"snappy"` | `"snappy"`, `"zstd"` or `"uncompressed"` |
| `compression_level` | unset | ZSTD level 1 to 22 (default 1); refused for other codecs |
| `row_group_rows` | 65536 | Maximum rows per row group |
| `layer_row_groups` | `true` | A row group also ends at every layer boundary, so a `layer` filter skips whole row groups. `false` fills row groups to `row_group_rows` across layers (pyarrow `write_table(row_group_size=N)` layout) |
| `dictionary` | `"all"` | `"all"`: the Parquet writer may dictionary-encode every column. `"utf8"`: only top-level Utf8 columns (the layer column included); all others are plain encoded, as pyarrow `use_dictionary=[utf8 names]`. Mostly unique numeric columns fall back from dictionary inside every row group, so `"utf8"` gives smaller files |
| `bbox_covering` | `"none"` | `"struct"`: a nullable `bbox` struct (`xmin`, `ymin`, `xmax`, `ymax`) with covering paths `["bbox", "xmin"]`, the GeoParquet 1.1 layout. `"columns"`: four float64 columns `bbox_xmin` ... `bbox_ymax`, the layout of ras2cng archives up to 0.12; DuckDB and pyarrow filter on them, but GeoParquet 1.1 validators reject their one-element covering paths |
| `order` | `"input"` | `"hilbert"`: within each layer, the Hilbert index of each bbox centre over the layer's extent, exactly DuckDB spatial's `ST_Hilbert(x, y, extent)` (16 bits per axis); ties keep input order, null and empty geometries go last |
| `layer_column` | unset | Name of a nonnullable Utf8 column holding each input's layer name; required with more than one layer, and then every layer needs a unique, nonempty name |
| `identity` | `{}` | Per layer name, `{"namespace": <Utf8 column>, "local": <Int64 column or null>}`; unlisted layers use `(mesh_name, cell_id)`. Names of layers that are not supplied refuse |
| `hilbert_column` | unset | With `order` `"hilbert"` only: name of a nullable UInt64 column appended after the bbox covering, holding each row's Hilbert key (the sort key, over its layer's extent); null for null or empty geometry. Refused with `order` `"input"` |
| `contract_metadata` | `false` | Keep the input-side `ras:export:v1` key in the output's Arrow schema metadata. By default it is dropped: the `geo` metadata carries the CRS |
| `max_batch_rows` | 65536 | As for the other writers |

Attribute types are wider than contract v1's, because Parquet stores and returns them
unchanged through the Arrow schema it embeds: the contract v1 scalars, Float32 (never
widened), Timestamp in milliseconds, microseconds or nanoseconds (time zone kept as
given), an all-null `Null` column, and List, LargeList, FixedSizeList and nonempty Struct
nested to any depth over Int64, UInt64, Float64, Float32, Boolean, Utf8, Binary or those
timestamps (for example `list<list<double>>` station/elevation pairs, or
`list<struct<Station, Elevation>>`). NaN and infinity refuse in float columns and in
float values nested in lists or valid struct rows. Other types refuse with the field name:
narrower integers, Float16, dates, second timestamps, dictionaries, LargeUtf8,
LargeBinary, maps, unions, empty structs and nested `Null`. The other writers keep the
contract v1 types.

The output's `geo` metadata is deterministic: `geometry_types` is sorted by name, as
GeoPandas writes it, and the same input and options give a byte-identical file.

Each layer is validated against contract v1 with its own schema and identity. All layers
must carry the same `ras:export:v1` CRS evidence and the same geometry field. The output
schema is the union of the layer fields in first-appearance order, then the layer column,
then the bbox covering. A field missing from a layer is null for its rows; a field with
different types in two layers, or one that collides (case-insensitively) with a generated
column, refuses. Bbox values are the minimum and maximum over every coordinate, holes and
parts included, and null (not NaN) for null or empty geometry. Rows, attributes and WKB are
otherwise passed through unchanged. Unordered export streams one batch at a time;
`"hilbert"` holds one layer at a time.

## Lifecycle and memory

Writers validate and consume one batch at a time. Each operation stages inside a
dedicated temporary sibling directory; malformed later input, iterator failure,
writer error or finalization failure removes only that directory. Finalized files
are synchronized and installed using no-clobber persistence. Existing targets
reject. This is single-file atomic installation, not crash-proof multi-file
transactionality or an append/multi-layer API. GeoPackage currently creates one
`features` layer per output. Python IPC spool size is proportional to total input.

Parquet buffers its configured row group. FlatGeobuf's official writer spools
features to disk and retains feature offsets/index nodes; indexed FGB memory is
not constant in total feature count. Do not claim whole-pipeline bounded memory
merely from a bounded input batch. The source Python table may also be eager.
Metrics separate validation, writer calls, finalization and total time. Encoding,
compression and index work internal to dependency calls is grouped, not falsely
reported as independently measured stages.

## Build and dependencies

From the repository root:

```powershell
cargo test --locked --features python,exports-all,exports-pmtiles
cargo clippy --locked --all-features --all-targets --no-deps -- -D warnings
```

ras-commander's `tests/rust_first_tier_exports` suite (with its harness crate) is
the end-to-end qualification of the writers.

To build the Python extension with the export features (the Python-side
registration of the export bindings lives in the extension's `exports-*` features):

```powershell
uv pip install maturin pyarrow pyproj shapely
cd python
uv run maturin develop --release --locked --features extension-module,exports-all,exports-pmtiles
```

`exports-arrow` (implied by every writer feature) gives Arrow IPC only. Opt in
to writers with `exports-geojson`, `exports-flatgeobuf`, `exports-geoparquet`,
`exports-geopackage`, `exports-all` (all four) and `exports-pmtiles`. Basic HDF builds gain
no mandatory GDAL or SQLite dependency. The GeoPackage feature bundles SQLite C
and needs a C toolchain. This writer crate needs no GDAL; GDAL/pyogrio are
independent qualification readers. Python reprojection uses pyproj/PROJ. The
embedded `freestiler-core` (feature `python`) brings its own compression
dependencies.

Direct pins: Arrow/Parquet 59.0.0 (Apache-2.0), GeoParquet 0.9.0 and
GeoZero 0.15.1 (MIT OR Apache-2.0), FlatGeobuf 6.0.1 (BSD-2-Clause),
GeoPackage 0.10.0 (MIT OR Apache-2.0). Lockfiles pin transitive resolution.
This is not a full legal redistribution audit. Release wheels include
`exports-all` and `exports-pmtiles`, and `python/licenses/THIRD-PARTY-NOTICES.md` covers
their dependency graph. The writers need Rust 1.95 or later and, because every
`exports-*` feature implies `python`, PyO3.
