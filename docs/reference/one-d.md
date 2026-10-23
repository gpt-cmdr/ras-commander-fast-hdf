# 1D geometry, 1D results and 1D structures

This page describes the engine's exact readers for one-dimensional HEC-RAS data: cross
section geometry and hydraulic property tables, river and line geometry, cross-section
results, 1D Summary Output, and 1D structures and pump stations. These cover the 1D HDF
families that ras-commander reads with h5py (`HdfXsec`, `HdfHydraulicTables`,
`HdfResultsXsec`, `HdfResultsPlan`, `HdfStruc`, `HdfStruc1D`, `HdfPump`). The readers were
added in engine API 7; no existing signature or behaviour changed. Writers stay on h5py.

## Common rules

 values keep their stored numeric type (f32 stays f32, i32 stays i32);
compounds keep their exact raw bytes beside decoded columns; nothing stored is recomputed.
Canonical absence is Unavailable (or `NotFound` from direct reads), unproven layouts are
Unsupported with a typed code, contradictory storage is Corrupt. `metadata_max_bytes` /
`max_bytes` keep the API 5 meanings (per consulted object / returned bytes).

## Cross-section geometry (`hdf::xs_geometry`)

- `xs_geometry_inventory(max_bytes)` returns the validated cross-section identities
  (`cross_sections`), the full `Geometry/Cross Sections/Attributes` compound with raw bytes
  (reach lengths `Len Left/Channel/Right`, banks, levees, `Contr`/`Expan`, HTAB parameters,
  friction mode, block modes), and one `XsGeometryDataset` per `XsGeometryField`:
  station-elevation, Manning's n (station, n), ineffective areas (compound blocks), deck
  high/low (5.x), hydraulic property tables (`Property Tables/XSEC Info`/`XSEC Value`, the
  HTAB `HdfHydraulicTables` reads) and per-cell property tables (`Cell Info`/`Cell Value`).
  Each field has its own capability, stored shapes/kinds, typed Info/Values attributes and
  column labels (`Variables`/`Variable_Unit` name+unit pairs, or `Column` names).
  Every Info range is validated against its Values array before Supported is reported.
  `unqualified` lists every other dataset below `Geometry/Cross Sections` (for example
  `Flow Distribution/*`, `Orthogonal Vectors`) as Unsupported(not_implemented).
- Separated 5.x legacy/hybrid layouts (`Node Names`, `River Stations`, `River Names`,
  `Reach Names` present) are Unsupported(identity_layout): they are never mixed with the
  compound identity table.
- `records_are_sections` is true only when Info rows equal the section count; then
  `section_records` equals that count and record i is section i. One trailing extra XSEC
  Info row was observed in HEC-RAS 6.6 plans that contain one inline structure (BaldEagle
  179 for 178 sections, Inline_3Gates 36 for 35, OrleansEx 336 for 335; bridges add no
  rows). The leading rows 0..n-1 are the sections: the first stored HTAB elevation of row i
  equals the minimum stored station elevation of section i exactly in all three files
  (178/178, 35/35, 335/335), so ras-commander's `get_xs_htab(xs_index)` (row == section
  index) reads the right table. The engine assigns those leading rows to the section
  Attributes rows (`section_records = n`) only after re-checking that exact elevation
  identity on the file; the trailing row(s) keep their XSEC Info row identity, because
  what they describe (and the layout with several inline structures) is not qualified by
  any available fixture. A failed check assigns nothing (`section_records = 0`).
- `read_xs_geometry(field, records, metadata_max_bytes, max_bytes)` returns the exact
  selected Info rows, the selected Values rows packed in record order (unsorted/repeated
  selections kept) and per-record `Identity` + absolute stored row range + packed offset.
  `max_bytes` bounds returned Info+Values bytes; a dense selection reads one covering slab
  when it fits the remaining budget, otherwise one slab per record.
- `one_d_line_family(family, max_bytes)` reads packed Info/Parts/Points for cross-section
  cut lines, river centerlines, bank lines, edge lines, flow paths (`Flow Path Lines`),
  structure centerlines and storage-area polygons, validating every feature/part range.
  Identity is the Attributes row when the family has one (required for cut lines,
  centerlines, structures, storage areas), else the Info row.
- `river_reaches(max_bytes)`: stored reach topology (River/Reach names, upstream and
  downstream type/name) from `Geometry/River Centerlines/Attributes`; no connectivity is
  inferred. `river_junctions(max_bytes)`: `Geometry/Junctions` Attributes + Points.
- `xs_interpolation_surfaces(max_bytes)`: exact TIN Info/Points/Triangles with validated
  point/triangle ranges per surface plus `XSIDs`, `Areas`, `Orthogonal Vectors`,
  `Bounding Lines`. Triangle index base is not asserted.

## Cross-section results (`hdf::xs_results`)

- `xs_result_inventory(mode, max_bytes)` enumerates every dataset below the canonical
  steady (`Steady Profiles/Cross Sections`) or unsteady (`Unsteady Time Series/Cross
  Sections`) group, recursively (steady `Additional Variables/*` included), excluding the
  identity tables. Names are relative paths (`Velocity Channel`,
  `Additional Variables/Area Flow Total`). Each `XsResultVariable` reports domain
  (`cross_section` = `(row, section)`, `cross_section_components` = `(profile, component,
  section)` such as steady `Cross Section Variables`, `other`), stored shape/kind/chunks,
  (rank 3 is Supported only when the stored `Dimension` attribute is exactly
  `[Profiles, Variables, Cross Sections]` in steady mode; otherwise Unsupported(schema)),
  every typed attribute with raw payload, the shared `dataset_units` evidence and declared
  `Variable_Unit` labels. Columns must match the results-side River/Reach/Station identity
  (validated exactly as `read_cross_section_result` does, compound or packed `Cross Section
  Only`); rows must equal the stored `Time` length (unsteady) or the steady profile count.
  An identity failure is the inventory capability and every variable's capability; a row
  mismatch is Corrupt; no matching section axis is Unsupported(identity_layout).
- `read_xs_result(mode, variable, rows, sections, metadata_max_bytes, max_bytes)` returns
  exact stored values for a stored row slice (time window or profile subset, strided
  allowed) and a section subset (caller order, repeats kept), row-major, with the geometry
  Identity of each selected column. `max_bytes` bounds returned values + identities and a
  transient covering slab; otherwise one column is read at a time.
- `unsteady_row_window(start_ms, end_ms, max_bytes)` maps a civil-time window (ms since the
  zone-less `CivilUnix1970` epoch of `unsteady_time_axis`) to the contiguous stored row
  slice; it never interpolates.
- Not covered: `Results/Post Process/Steady/...` profiles and DSS Hydrograph/Profile Output
  blocks (other output blocks are never substituted for Base Output).

## 1D and structure Summary Output (`hdf::one_d_summary`)

- `one_d_summary_inventory(max_bytes)` enumerates every dataset (recursively, depth 3) in
  every group directly below `Results/Unsteady/Output/Output Blocks/Base Output/Summary
  Output` except `2D Flow Areas` (qualified by real files: `Cross Sections`, `Storage Areas`). Any other family (for example
  a structure group) is enumerated with domain `other` but every variable is
  Unsupported(schema, "unqualified layout") and is not read, because no real fixture
  writes one; it is promoted only when a real file qualifies it. Canonical absence
  is Unavailable. Each `OneDSummaryVariable` carries family, relative name, stored
  shape/kind/chunks, every typed attribute, declared `Variable_Unit` (6.x) or `Variables`
  (5.0.7) name/unit pairs, proven per-row labels and `time_of_extreme_rows`.
- Element axis: the axis whose length equals the element count (cross sections: the
  validated unsteady results-side identity count; storage areas: `Geometry/Storage
  Areas/Attributes` rows). Rank-1 counters (`Last Iteration`, `Maximum Iteration`) use axis
  0; XS-major tables such as `Maximum Velocity Flow Distribution (sections, 23)` use axis 0;
  `(rows, sections)` extremes use axis 1. When both axes match, declared per-row labels
  decide; otherwise the variable is Unsupported(identity_layout) for element selection
  and readable only whole. Storage-area column order is NOT asserted, so storage-area
  selections carry no element Identity.
- Row labels are proven from `Variable_Unit`/`Variables` pairs whose count equals the stored
  row count, from `Row0..RowN` (+ shared `Units`), or from `Row N:  name` = unit attributes
  (storage areas). `time_of_extreme_rows` are rows whose label starts with `Time` (for
  example `Time at Max WSEL`, row 2 of `Maximum Water Surface`); the stored values are days
  exactly as written, never converted to timestamps.
- `read_one_d_summary(family, name, elements, metadata_max_bytes, max_bytes)` returns every
  stored row with the element axis subset (caller order, repeats kept) and geometry
  identities for the cross-section domain.

## 1D structures and pumps (`hdf::structures_1d`)

- `structure_geometry(max_bytes)`: one `StructureRecord` per `Geometry/Structures/
  Attributes` row (exact `Type`, `River`, `Reach`, `RS`, `Connection`, `Node Name` as
  `Reported` text) with `Table Info` `X (Index)`/`X (Count)` pairs validated as ranges into
  `Profile Data` (centerline, US/DS XS, bridge deck/weir/lid profiles, Manning's n, rating
  curves), the Attributes compound with raw bytes and the exact group dump. 5.0.7 files
  without `Table Info` (JUL2020: `Profiles` compound instead) keep empty ranges.
- `one_d_geometry_group(group, max_bytes)` for `Structures`, `PumpStations`,
  `StorageAreas`: every compound (bridge coefficients, piers, abutments, gate groups and
  openings, culvert groups and barrels, weir connectivity, pump groups, pumps, advanced
  rules, storage-area attributes) with raw bytes and every numeric array (profile/pier/
  abutment data, centerline packed arrays, structure property tables, efficiency curves,
  storage-area elevation-volume Info/Values) with stored kinds. One per-object policy:
  any dataset (compound or numeric) or group-attribute set that is not returned is listed
  in `unsupported` with its own capability (unproven encoding, byte budget as
  Unsupported(resource_limit), damaged storage as Corrupt); only host errors and the
  depth/object limits fail the whole group. A present but malformed `Connection` column in
  the identity table is surfaced (Unsupported), never replaced by empty text, and
  `read_one_d_structure_series` keeps Corrupt distinct from Unsupported (with its code). `pump_stations(max_bytes)`: exact Attributes + Points + names.
- `one_d_structure_inventory(kind, mode, max_bytes)` for `OneDStructureKind` =
  inline structures, bridges, culverts and pumping stations (unsteady Base Output) and steady
  lateral structures / storage areas: the layouts qualified by real files
  (`OneDStructureKind::qualified`). The same group names under steady profiles (no available
  steady plan writes them) are Unsupported(identity_layout, "unqualified layout") and are
  not read; the reader code stays and is promoted only when a real fixture qualifies them. Unsteady lateral
  structures and storage areas remain the qualified `structure_inventory` families
  (InvalidRequest here). Identity: the exact group name equals the space-joined trimmed
  `River Reach RS` of exactly one Attributes row with the matching `Type` (`Inline`,
  `Bridge`, `Culvert`, `Lateral`), or the exact `Name` of the pump-station/storage-area
  table; unmatched or non-unique keys are Unsupported(identity_layout), duplicate groups and
  duplicate `Node Pointer` values Corrupt. Series follow the `hdf::structures` conventions:
  rank-2 datasets (rows = time axis or steady profile count, else Corrupt), labels from
  `Variable_Unit`/`Connections`, dataset units from `Units` only, `Geometric Info` skipped,
  rank-1 datasets listed as static. Gate groups (`Gate Groups/<name>`), bridge, culvert,
  pump (`Structure Variables` incl. `Pumps on`), lateral `HW TW Segments` and storage-area
  `Connections to Storage Area` series are all covered. Steady storage-area family datasets
  (`Flow`, `Water Surface`) are family series; their column order is not asserted.
- `one_d_structure_capabilities(max_bytes)` isolates each (kind, mode) family status;
  `read_one_d_structure_series(kind, mode, structure, relative, rows, columns,
  metadata_max_bytes, max_bytes)` returns exact f32/f64 slices.
- `plan_capabilities()` is unchanged: its aggregate `Structures` entry still reports these
  groups as unqualified (Unsupported) for API 6 compatibility; inspect
  `one_d_structure_capabilities` for their status. DSS Hydrograph/Profile Output copies of
  these families are not read (other output blocks are never substituted for Base Output).

## Layer catalogue

`layer_inventory(max_bytes)` now also lists, before the plan result-family reports:
`xs_geometry` (one layer per `XsGeometryField` with Info/Values fields; domain
`cross_section` only when `records_are_sections`, plus an `unqualified` layer),
`one_d_lines` (seven line/polygon families), `one_d_topology` (`river_reaches`,
`river_junctions`, `xs_interpolation_surfaces`), `xs_results` (`<mode>/<variable>`),
`one_d_summary` (`<family>/<variable>`), `structure_1d_results` (one series layer per
structure with its geometry Identity, family-level layers, and isolated statuses per
`<kind>/<mode>`), `structure_1d_geometry` (`structures`, `pump_stations`) and
`one_d_geometry_group` (exact table/array fields). An enumerated but empty family is
Unavailable. Non-host failures stay per-layer capabilities (an uncoded InvalidRequest is
reported Corrupt, as in the geometry-layer catalogue of [engine-api.md](engine-api.md)); `layer_inventory(0)` still succeeds.

## Oracles

The 1D conformance tests are named `wave2_1d`.
`tests/fixtures/wave2_1d/oracle_wave2_1d.py` (h5py, independent of the engine) writes a
manifest with source SHA256s, shapes, dtypes and SHA256 of the exact little-endian bytes the
engine must return, plus deterministic unsorted/repeated samples. `tests/wave2_1d.rs` is
`#[ignore]` and enabled by `RAS_WAVE2_1D_ORACLE` (manifest path); setting the variable to a
missing file fails explicitly. The manifest is relocatable: it stores a `base` directory and
fixture paths relative to it with SHA256s; set `RAS_WAVE2_1D_FIXTURE_ROOT` to the copied
fixture tree on another host (a missing fixture fails with its resolved path; every file is
re-hashed). The oracle re-derives values and the documented selection rules from raw
datasets; its classification checks (element axes, readability) restate the engine's
documented rules, so only the value SHA256s are fully independent of the engine.

`tests/fixtures/wave2_1d/make_fixtures.py` generates six small synthetic files (SHA256
manifest) that run by default in `tests/wave2_1d_synthetic.rs`: incomplete Info/Values pairs
and out-of-range Info rows are Corrupt, float Info Unsupported(encoding), separated XS
layouts Unsupported(identity_layout), row/time disagreement Corrupt, missing section axes
Unsupported(identity_layout), unmatched structure identities Unsupported(identity_layout),
family-level stray datasets Corrupt and absent families Unavailable; exact NaN/-0.0 bits
and unsorted/repeated selections are checked. Also covered: unqualified steady bridge results
and a structure Summary Output family (Unsupported, not read), rank-3 results without
`Dimension` (Unsupported) and a two-profile rank-3 gather on both read paths, an integer
`Connection` column, one budget policy for over-budget tables and arrays, multi-part lines
(stored Parts relative to the feature, returned absolute) and bad part/TIN ranges (Corrupt).
