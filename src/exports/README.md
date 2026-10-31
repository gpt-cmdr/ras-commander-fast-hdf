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

Attributes are nullable and of these types:

| Arrow type | Arrow IPC, GeoParquet | FlatGeobuf, GeoPackage, GeoJSON |
| --- | --- | --- |
| Int64, UInt64, Float64, Boolean, Utf8, Binary, timezone-free Timestamp(Millisecond) | Exact | As in the format profile below |
| Float32 | Exact (Parquet `FLOAT`) | Exact: FlatGeobuf `Float`; GeoPackage `FLOAT` column stored as SQLite REAL (the float32's exact float64 value); GeoJSON the exact float64 value (2.6f32 is `2.5999999046325684`) |
| Null (an all-null column) | Exact (Parquet `UNKNOWN`) | Refused |
| Timestamp(Microsecond or Nanosecond), timezone-free | Exact (`TIMESTAMP(MICROS/NANOS, false)`) | Refused |
| List of any type in this table, nullable elements | Exact, nested nulls, empty lists and offsets kept; Parquet 3-level `LIST`, element named `element` (the specification's and pyarrow's layout) | Refused |
| Struct of types in this table, nonempty, unique nonempty names, nullable children | Exact | Refused |

Refused formats refuse before any output is staged, naming the field; convert such
columns explicitly first. Dictionary, LargeList, FixedSizeList, Map, LargeUtf8/LargeBinary,
narrower integers, Timestamp(Second), timezone-qualified timestamps, non-nullable list
elements or struct children, and Null-typed fields declared non-nullable reject everywhere.
Time means UTC Unix time in the stated unit; dates outside a container's supported
calendar reject there. Timezone normalization is a producer operation. NaN/infinity reject
at any nesting depth; supply Arrow null explicitly. Empty/NUL, exactly duplicated and
reserved `fid`/`_ras_fid` field names reject. Names that differ only in case (`River` and
`river`) are kept by Arrow IPC and GeoParquet and rejected by FlatGeobuf, GeoPackage,
GeoJSON and the PMTiles bridge, whose consumers treat column names case-insensitively.
Arrow schemas must remain exactly equal, including metadata and nested types, and each
input batch must satisfy `max_batch_rows`.

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
| Arrow IPC file | Original WKB, schema and metadata | Every contract type exactly, nulls and full uint64 |
| GeoParquet | WKB GeoParquet metadata, native PROJJSON, bbox/type finalization | Every contract type exactly, Snappy, 65536-row groups, root group `schema` |
| FlatGeobuf | Official writer, native WKT, spatial index, mixed geometries; empty/null rejects | uint64 converts to checked signed64; larger values reject because GDAL otherwise rounds ULong to float64. Time stored as integer milliseconds, schema metadata carries contract. Null properties omitted in feature storage and reopen as null |
| GeoPackage | Complete SQLite container, feature table, SRS/WKT2 extension, RTree and contents tables | SQLite signed64 (larger uint64 rejects), bool, real, UTF8, blob, nullable fields; timestamp stored as UTC ISO text with millisecond precision. Field metadata beyond CRS is not archived |
| GeoJSON / Lines | WGS84 longitude/latitude only, CCW exterior/CW holes; Python transforms and orients | Nullable scalar properties; integers outside ±(2**53-1) become decimal strings. Millisecond timestamps remain JSON integers. Binary attributes reject. No full Arrow schema/field metadata archive. Lines is one feature per newline, with no RFC8142 record separator |

GeoJSON timestamp integers outside JavaScript's exact integer range require a
consumer with an exact integer parser; year3000 timestamps are inside that range.
GeoJSON has no fidelity claim for the original Arrow type schema. Binary source
exports retain geometry coordinates, without rendering simplification or tiling.
PMTiles/freestiler remains the separate rendering path and owner.

## GeoParquet archive profile (moved to terrio)

The multi-layer archive profile (`export_geoparquet_ipc_files`: layer column, bbox
covering, Hilbert order, ZSTD level, Utf8 dictionaries, chunk statistics) was removed in
0.3.0. terrio owns file formats: use `terrio::geoparquet::write_arrow_layers` (Python
`terrio.write_geoparquet_layers(..., profile="archive")`), which writes the same rows,
order, schema, row groups and `geo` metadata. The engine's Arrow output (arrow 59) passes
to it without conversion.

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
