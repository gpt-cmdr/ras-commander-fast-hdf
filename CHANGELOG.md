# Changelog

## Unreleased

- 1D summaries whose two axes have the same length (for example `Summary Output/Storage
  Areas/Maximum Water Surface (2, 2)` in a plan with exactly two storage areas) now take
  the element axis from stored `Row N:  name` (or `RowN`) attributes when these label
  exactly rows `0..len-1`. Before, only `Variable_Unit`/`Variables` declarations broke
  the tie and these variables were `Unsupported(identity_layout)`. Proven on HEC-RAS
  7.0.1 Beaver_LS_SAConn p01 (corpus `hec701r/Beaver_LS_SAConn.p01`): identical to h5py;
  no other summary variable in the 147-file corpus changes capability. Behaviour change
  within API 11.

## 0.2.4 (2026-10-28)

- `ReadOnlyHdfSession::hec_ras_file_type` classifies the stored root `File Type`
  attribute against `HEC_RAS_FILE_TYPES` (`HEC-RAS Geometry`, `HEC-RAS Results`):
  `HecRas`, `Other` (for example `HEC Terrain`) or `Absent`; a value that is not a
  single string is refused. It lets callers such as terrio's HECRAS driver claim only
  HEC-RAS geometry and plan files. Rust only; additive within API 10.
- Pipe networks (Rust only, additive within API 10): `OneDLineFamily::PipeConduits`
  (`Geometry/Pipe Conduits`, listed in the new `PIPE_LINE_FAMILIES`, not in
  `ONE_D_LINE_FAMILIES`) and `ReadOnlyHdfSession::pipe_nodes` (`Geometry/Pipe Nodes`).
  Both use the existing packed-line and point-family rules.
- Extensible-array chunk indexes with more elements than their index block
  (HDF5 1.10+ files written with `libver="latest"`, chunked along an unlimited
  dimension) are read instead of refused. hdf5-reader 0.9.3 returned fill values
  for their chunks past the index block, so the engine refused them (`Unsupported`,
  "non-inline extensible-array chunk index"); 0.10 maps them as libhdf5 does.
  Every value of `tests/fixtures/reader_policy/ea_latest.h5` (50 chunks) and the
  new `ea_deep.h5` (9,000 chunks: super blocks and paged data blocks) matches
  h5py, in full reads, slices and row/column gathers. HEC-RAS does not write
  this layout. Behaviour change, separate from the reader update.

- The HDF5 reader is now the crates.io `hdf5-reader`/`hdf5-core` 0.10.1
  (roteiro-gis/netcdf-rust), a normal dependency, instead of a patched copy of
  0.9.3 compiled into the crate. Upstream 0.10.0 and 0.10.1 contain 12 of the 14
  local patches (#102 to #109 and #111; the mapping is in `docs/reference/provenance.md`). The
  other two were CLB policies and are now applied by the engine itself, in
  `src/hdf/reader.rs`, with the same refusals, error kinds, codes and messages:
  a pipeline with LZ4, SZIP or an unknown filter as an intermediate decoding
  stage is refused (`Unsupported`, `encoding`), and so is an extensible-array
  chunk index with more elements than its index block. The policies are now
  judged per dataset rather than per chunk, so they are slightly stricter
  (no HEC-RAS file is affected). Engine outputs on every committed fixture are
  unchanged; five malformed-attribute fixtures keep their error kind and code
  but get upstream's wording ("reserved flags 0x80 in attribute v2" for
  "reserved attribute flags", "shared datatype/dataspace in attribute v2 is not
  supported" for "shared attribute encoding", and the size in "datatype
  description of 12 bytes exceeds its 8-byte message").
  0.10.1 adds #111: row/column gathers look up only the selected chunks in
  v1/v2 B-tree, fixed-array and extensible-array indexes, which the old patch
  0013 did with one bounded query (same values; sparse gathers do less index
  work). Upstream 0.10.0 also reads paged fixed-array chunk indexes correctly (bit
  order and checksum of the page bitmap, `e18dfde`) and numbers
  extensible-array chunks by the maximum dimensions (#105), so values of such
  datasets (not written by HEC-RAS) can differ from the old reader.
- `vendor/` now holds only `freestiler-core`. `vendor/hdf5-*`, `vendor/patches`,
  `vendor/reader-tests` and `vendor/LOCAL_PATCHES.md` are removed;
  `scripts/vendor_sources.py` verifies and rebuilds freestiler only. The crate's
  licence expression is `MIT AND ISC` (the reader's `MIT OR Apache-2.0` now
  applies to the dependency, listed in `THIRD-PARTY-NOTICES.md`). The whole
  crate is `#![forbid(unsafe_code)]`.

- Export contract v1 accepts the column types of ras2cng's archives. Arrow IPC and
  GeoParquet (plain and archive profile) store Float32, `Null` (all-null) columns,
  timezone-free Timestamp(Microsecond/Nanosecond), and `List` and `Struct` of supported
  types at any depth exactly: values, nulls, nested nulls, empty lists and offsets, with
  the Parquet 3-level `LIST` layout and element name `element`, as pyarrow writes it.
  FlatGeobuf (`Float`), GeoPackage (`FLOAT`, stored as REAL) and GeoJSON (exact float64
  value) store Float32 exactly; they refuse the other new types before writing anything.
  NaN and infinity now reject at any nesting depth. Field names that differ only in case
  (`River`, `river`) are kept by Arrow IPC and GeoParquet; FlatGeobuf, GeoPackage, GeoJSON
  and the PMTiles bridge still refuse them. Large/fixed-size lists, maps, dictionaries,
  narrower integers and timezone-qualified timestamps still reject.
- GeoParquet archive profile options `dictionary` (`"all"`, `"utf8"`, `"none"`),
  `hilbert_column` (store the Hilbert key as a nullable UInt64 column), `statistics`
  (`"page"`, or `"chunk"` for pyarrow's column-chunk statistics without a page index) and
  `geometry_statistics`. Both GeoParquet writers now name the Parquet root group
  `schema`, as pyarrow does. With ras2cng's options, on 17 archive files from five
  HEC-RAS 7.0.1 example projects (155 to 264,351 rows), ras2cng's Python writer and this
  profile give identical tables on read, Parquet schemas, GeoParquet metadata, row
  groups, codecs, dictionaries and statistics; the bytes differ (see
  `src/exports/README.md`).
- Fix: the archive profile's file-level GeoParquet `bbox` was `[null, null, null, null]`
  when any geometry was empty. It is now the extent of the non-empty geometries.
- The archive profile reorders a one-batch layer with `take` instead of `interleave`,
  which is faster for list and struct columns.

- `write_pmtiles` property columns can be NumPy arrays (`float64/32`, `int64/32`,
  `uint64/32`, `bool`), `(values, valid)` pairs with a boolean mask, or categorical
  `{"codes", "categories"}` string columns, read in Rust with no Python object per value.
  The optional `property_rows` (one source row per feature, `-1` for null) lets callers pass
  whole source-indexed result arrays. The archive is byte-identical to the list form. On a
  340,000-cell layer with 10 float columns plus `mesh_name`/`cell_id`, the Python
  preparation drops from 2.9-3.4 s to 0.01 s, and peak working set by about 100 MiB.
  Lists still work. Holes and multipart features (`ring_offsets`/`polygon_offsets`/
  `feature_offsets`) now have a decoded-tile test: exterior, hole and island keep their
  windings.
- GeoParquet archive profile fixes, found by writing a real ras2cng archive
  (BaldEagleDamBrk: 15 geometry files and the p07 results):
  - `geometry_types` in the `geo` metadata is sorted by name, as GeoPandas writes it.
    Before, it followed `HashSet` order and changed between runs. Output is now
    byte-identical for the same input and options.
  - The output no longer carries the internal `ras:export:v1` key in its Arrow schema
    metadata; `contract_metadata: true` keeps it. The embedded Arrow schema now carries
    the `geo` metadata, as pyarrow's writer does, so pyarrow `read_schema` sees it.
  - The file-level `bbox` is omitted, not `[null, null, null, null]`, when no row has a
    geometry.
  - New option `layer_row_groups` (default `true`; `false` fills row groups across
    layer boundaries, the pyarrow `write_table(row_group_size=N)` layout).
  - The `row_groups` metric is counted at close, so it includes a row group that is still
    open then (possible with `layer_row_groups: false`).
  - Unsupported-type refusals name the field. NaN or infinity hidden under a null list
    or struct slot is no longer refused: only values a valid row references are data.
- GeoParquet archive profile (`exports-geoparquet`): new
  `export_geoparquet_ipc_files(inputs, output, options)` writes one GeoParquet from one
  or more Arrow IPC layers, with ZSTD (level selectable), a row-group size, per-row bbox
  covering, Hilbert order within each layer and a `layer` discriminator column. Layers
  are validated against contract v1 on their own schemas and unified into one schema
  (a field missing from a layer is null there; a field with different types refuses).
  Covering is either the GeoParquet 1.1 `bbox` struct (`"struct"`, recommended,
  which geoparquet-io's spec check accepts) or ras2cng's four `bbox_xmin` ...
  `bbox_ymax` columns (`"columns"`), which DuckDB filters on but spec validators reject
  because their covering paths have one element. Hilbert order is DuckDB spatial's
  `ST_Hilbert` of each bbox centre over the layer's extent, bit for bit; null and empty
  geometries go last and ties keep input order. A row group ends at every layer
  boundary. Unordered export streams one batch at a time; Hilbert order buffers one
  layer at a time. The plain `parquet` format is unchanged (Snappy, 65,536-row groups).
  On the BaldEagleDamBrk g04 geometry (264,346 rows in 7 layers), against ras2cng 0.12's
  `_write_geoparquet`: identical rows, attributes and WKB; writer time 2.8-4.1 s to
  1.5-2.1 s (0.4-0.6 s native, the rest the Python Arrow adapter); peak working set
  1,460 to 855 MiB.
- Export contract v1 identity is now selectable per layer for the GeoParquet profile:
  a Utf8 namespace with an optional Int64 local ID (`Identity`), so mesh faces
  (`mesh_name`, `face_id`) and one-row-per-name layers (`mesh_name`) can be exported.
  The default stays `(mesh_name, cell_id)` and the other writers are unchanged.
- `parquet` now builds with its `zstd` feature (adds `zstd` 0.13 and `zstd-safe` 7 to
  the lock file; `zstd-sys` was already linked).
- Faster reference-line result reads. Since 0.2.2 every `read_family_dataset` of a
  `ReadFamily::ReferenceLines` dataset proved the column-to-geometry link again (it read
  the whole reference-line geometry family), although the link never changes what the
  read returns. Value reads no longer prove it; inventories still report it, and each
  Name axis's link is now proven once per session (the per-dataset `2D Areas` check
  still runs on every call). Output-block time rows (`family_time_rows`) are also proven
  once per session. On a 124-line plan a session read drops from about 4.5 ms to
  0.1 ms. Values, errors and capabilities are unchanged; errors are never memoized.
- Python extension: lists of identities (for example `read_xs_result`'s `section_ids`,
  `xs_result_inventory`'s `sections`, family `locations`) convert each run of equal
  `source`/`table` once and share the equal, immutable `pathlib.Path`/`str` object,
  instead of building one `pathlib.Path` per row.
- No API change: engine `API_VERSION` stays 10 and extension `API_VERSION` stays 11.

## 0.2.3 (2026-10-26)

- New read family `mesh_corner_elevations` (`ReadFamily::MeshCornerElevations`): the
  static ground elevation of every 2D mesh corner (face point), which RASMapper's
  depth-weighted sloping interpolation uses. Two stored layouts are read exactly, as
  rank-one little-endian `f4` arrays with one value per `FacePoints Coordinate` row:
  `Geometry/2D Flow Areas/<mesh>/FacePoints Elevation` (geometry or plan HDF) and the
  RASMapper mapping cache `.../Processed Data/Profile (Horizontal)/FacePoint Elevation`
  (`PostProcessing.hdf`). Values are returned bit for bit, NaN payloads and sentinels
  included. A cache stores no coordinates, so it is qualified only by the typed reader
  with the plan's corner count. Evidence: 41 geometry arrays in 35 HEC-RAS 7.0/7.0.1
  files and two native RASMapper 7.0.1 caches, all bit-identical to h5py.
- `ReadOnlyHdfSession::mesh_corner_elevations(mesh, layout, expected_corners, max_bytes)`
  reads one mesh in one layout with its provenance (file identity, dataset path, shape,
  attributes, corner count and its source). Absent data is Unavailable, never an empty
  array; a corner-count disagreement is Corrupt.
- The Python extension gains one entry point, `mesh_corner_elevations` (additive).
  Engine `API_VERSION` stays 10 and extension `API_VERSION` stays 11; consumers detect
  the entry point with `hasattr`.
- `ReadOnlyHdfSession::read_mesh_face_lines(mesh, max_bytes)` and the Python
  `HdfSession.read_mesh_face_lines(area, max_bytes=...)` return every face of a
  2D mesh as packed polylines (GeoArrow linestring layout: float64 `(N, 2)`
  coordinates, int64 offsets, face IDs). They read only the four face tables and
  reuse `Topology::face_lines`, so each line is ras-commander's Python
  `HdfMesh.get_mesh_cell_faces` line (start point, `Faces Perimeter Values` rows,
  end point). Layouts the Python per-face loop reads differently are refused:
  tables that are not `(rows, 2)` (Unsupported); unequal face tables, negative or
  out-of-range indices and perimeter spans, nonfinite coordinates
  (StructuralTopology). The packed output is budgeted before it is built.
  ras-commander builds all lines of a mesh in one `shapely.from_ragged_array`
  call from these buffers instead of one `LineString` per face.
- Additive: extension `API_VERSION` stays 11 and engine `API_VERSION` stays 10.
  ras-commander detects the method with `hasattr(HdfSession, "read_mesh_face_lines")`.

## 0.2.2 (2026-10-25)

- Reference-line results are linked to their geometry rows:
  `FamilyDataset::geometry_locations` is now filled for `ReadFamily::ReferenceLines`,
  matched by the exact stored `Name|SA-2D` and cross-checked against the geometry's
  `Internal Faces` and any `2D Areas` ordinals. A new `FamilyDataset::geometry_link`
  capability reports the link's status (Unavailable without reference-line geometry,
  Unsupported or Corrupt when the stored identities do not prove it); reading the
  results is unchanged. HEC-RAS 7.0.1's per-area series
  (`2D Flow Areas/<area>/Reference Lines`, bare names) are linked through the path's area,
  with the Faces cross-check against the same output block's `Reference Lines/Name` axis.
  Additive; the Python extension's `geometry_locations` for reference lines changes from
  empty to the linked rows.
- `ReadOnlyHdfSession::from_bytes(data, name, threads, parallel)` opens an HDF5
  file held in memory (a download or an upload). Reads are the same as for the
  file opened by path, with the same byte budgets and refusals; `name` stands in
  for the path in identities and error messages. The session owns the bytes, so
  the whole file stays in memory while it is open. This makes the engine usable
  in WebAssembly (terrio's JavaScript package), where there is no file system.
- Where the platform cannot start threads (WebAssembly without threads), a
  one-thread session runs its work on the calling thread instead of failing to
  build its worker pool. Results are unchanged; native builds are unaffected.
- Additive: engine `API_VERSION` stays 10 and extension `API_VERSION` stays 11.
  The Python extension and ras-commander do not call `from_bytes`.
- Qualified with the ras-commander equivalence gate (consumer 0.108.1, current
  manifests plus new HEC-RAS 7.0.1 reference-line/point fixtures): every result
  record is identical to 0.2.1's.

## 0.2.1 (2026-10-24)

- Test fixtures replaced with public sources; no API or behaviour change.
  The absent-Location legacy excerpts (`tests/fixtures/legacy_location`) are
  regenerated from the public FEMA eBFE Austin-Oyster plan `AustinOyster.p03`
  (HEC-RAS 5.0.7, area `Perimeter 1`). Source: FEMA estimated Base Flood
  Elevation (eBFE) program. The opt-in real-file tests now name
  `AustinOyster.p03` for 5.0.7 2D output and the eBFE-derived
  `Stream248.p02` (HEC-RAS 7.0.1) for 1D unsteady cross sections.
- Extension `API_VERSION` stays 11 and engine `API_VERSION` stays 10.

## 0.2.0 (2026-10-24)

- One Rust crate, `ras-commander-fast-hdf` (Rust import
  `ras_commander_fast_hdf`), at the repository root, ready to publish on
  crates.io. It replaces the engine crate `ras-commander-rust`, the separate
  extension crate in `python/`, the export crate `ras-export` and the vendored
  `hdf5-reader`/`hdf5-core` path crates. The name `ras-commander-rust` is
  retired. Rust users enable `hdf`, `mesh` and `results` as needed (default:
  `catalog`, standard library only); without the `python` feature no PyO3 or
  NumPy is in the dependency tree.
- The PyPI wheel is built from the same crate (`python/pyproject.toml`, features
  `python,extension-module`; release wheels add `exports-all,exports-pmtiles`).
  The Python API and behaviour are unchanged from 0.1.1; extension
  `API_VERSION` stays 11 and engine `API_VERSION` stays 10.
- HDF5 reader rebased from `hdf5-reader` 0.9.1 and its older local patch set
  onto upstream 0.9.3 (roteiro-gis/netcdf-rust `v0.9.3`) plus 14 local patches
  (`vendor/patches/`, described in `vendor/LOCAL_PATCHES.md`).
- `freestiler-core` (walkerke/freestiler `beb64b0`), previously a Git
  dependency, is embedded from `vendor/freestiler-core`. No Git dependencies
  remain.
- Embedded third-party source is compiled as private modules, recorded with
  hashes in `vendor/PROVENANCE.json` and checked by
  `scripts/vendor_sources.py --check`. Upstream's reader tests run from
  `vendor/reader-tests`.
- Layout: engine in `src/`, Python bridge in `src/python/` (was `python/src`),
  export writers in `src/exports/`, all tests in `tests/` with fixtures in
  `tests/fixtures`, `build.rs` at the root, notice scripts in `scripts/`.
- Licence notices: `THIRD-PARTY-NOTICES.md` (crate, all features) and
  `python/licenses/THIRD-PARTY-NOTICES.md` (wheel) come from one script,
  `scripts/generate_third_party_notices.py`. Inside the wheel the notices file
  is now under `.dist-info/licenses/licenses/` (same file name), because the
  sdist root already holds the crate's own notices.
- One name everywhere: the notices list the engine as
  `ras-commander-fast-hdf`, and the wheel's copy of this repository's licence
  is now `ras-commander-fast-hdf-LICENSE.txt` (was
  `ras-commander-rust-LICENSE.txt`). ras-commander 0.108.0 and later check
  these names.
- Path-based typed reads can share an open session. Every module-level
  function that takes an HDF path for a typed read (typed attributes, fixed
  strings, 1D line, geometry, result, summary and structure readers, 2D
  geometry families, stored-family and output-block readers) now also accepts
  an open `HdfSession` in place of the path. The call then reuses that
  session's file mapping, chunk cache and metadata memo instead of opening the
  file and repeating the layout and encoding proofs for each small read. The
  values are the same. A closed session raises `ValueError("HDF session is
  closed")`; a path behaves exactly as before. The module flag
  `SESSION_SOURCES = True` lets callers detect support. This is additive:
  `API_VERSION` stays 11.
- Cross-section geometry reads other than the property table no longer decode
  the whole HTAB value table (`Property Tables/XSEC Value`, stored by HEC-RAS
  with one row per compressed chunk) only to count its leading section rows.
  That count is used by the property-table field alone; it is now computed
  when that field or the full `xs_geometry_inventory` is requested. Results are
  unchanged. On the BaldEagle example the first cross-section read in a session
  drops from about 40-70 ms to under 2 ms.
- Engine (additive within API 10): `Topology::face_lines()` returns every 2D
  face as a polyline (start point, stored perimeter vertices, end point) and
  `read_mesh_perimeter()` returns an area's stored `Perimeter` as a closed ring,
  both exactly as ras-commander's Python `get_mesh_cell_faces` and
  `get_mesh_areas` build them (bit-identical on 20 public HEC example files).
  The extension does not call them yet; its API is unchanged.
- The engine no longer depends on `geo`. Cell-ring validation and winding now
  use the `robust` crate's adaptive-precision `orient2d` (the predicate `geo`
  used) and a port of `geo` 0.29's segment-contact classification. `geo` 0.29
  pinned `i_overlay`/`i_float` versions that conflict with `geo` 0.33, so Rust
  consumers that use `geo` 0.33 (terrio 0.5, RustGIS) could not link the
  engine. Results are unchanged: unit tests compare the predicates with
  `geo` 0.29 on every segment pair of a 4 x 4 grid and 800,000 random and
  near-collinear pairs, and packed polygons and omissions of 134 mesh/policy
  reads across 31 public HEC example HDF files are bit-identical to 0.1.1.
  Engine `API_VERSION` (10) and extension `API_VERSION` (11) are unchanged.
- Engine: HEC-RAS 5.0 Maximum/Minimum Water Surface summaries, which carry
  only `Row 0:  WSEL` = `Feet` and `Row 1:  Time` = `Days` attributes and no
  `Location`, now qualify as cell summaries (new `DomainRule::LegacyRowLabels`)
  instead of `unsupported(identity_layout)`. Values are the literal stored
  arrays, so ras-commander results do not change: before, it read the same
  values through its literal fallback. Any other layout stays refused.

## 0.1.1 (2026-10-24)

- Linux wheels for older and musl-based distributions: `manylinux2014`
  (glibc 2.17 or later) replaces `manylinux_2_28`, and a new `musllinux_1_2`
  wheel covers Alpine. `pip install ras-commander` no longer tries to compile
  the engine on those systems. Engine and extension code are unchanged.

- Documentation: this package is ras-commander's default HDF engine, not an
  optional extension. ras-commander 0.107.1 and later list it as a core
  dependency on Windows x64, Linux x64 and macOS arm64 (CPython), so
  `pip install ras-commander` installs it; on other platforms ras-commander
  uses h5py. `RAS_COMMANDER_FAST_HDF=h5py` still turns it off. The package
  itself is unchanged.

## 0.1.0 (2026-10-23)

First public release. The engine (previously the private
`ras-commander-rust` repository) and the Python extension (previously
`rust/fast-hdf` in ras-commander) now live together in this repository.

- Extension `API_VERSION` 11, engine `API_VERSION` 10. Works with
  ras-commander 0.107.0 and later.
- Native reads for ras-commander's plan results, geometry, terrain and
  land-cover HDF access: numeric, compound and fixed-length string datasets,
  attributes, 2D mesh geometry, summaries, time series and boundary
  conditions, cross sections and 1D results, structures, reference lines and
  points, terrain modifications and stored classification tables.
- Native 2D cell polygon assembly, fluvial/pluvial kernels, PMTiles and vector
  exports (GeoJSON, FlatGeobuf, GeoParquet, GeoPackage), and RAS text-file
  parsing.
- The wheel's `THIRD-PARTY-NOTICES.md` covers every bundled crate. `epsg-utils`
  (used by the GeoPackage writer) declares MIT but publishes no licence text;
  its notice reproduces the MIT text with an explanation.
- Qualified identical to h5py through ras-commander on 137 HEC-RAS 5.x–7.x
  files (3,800 method × file pairs in three modes) and 1,428 notebook calls,
  on Windows and Linux.
- `ENGINE_COMMIT` is now the commit of this repository the wheel was built
  from. Wheels built before the move report the private engine commit
  `0b024adc7857e9fb2d364997ad002e01a939cec1`, whose engine source is identical.
