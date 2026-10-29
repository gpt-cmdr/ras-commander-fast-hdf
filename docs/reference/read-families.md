# Stored read families

A read family is a group of related datasets in a HEC-RAS HDF file (for example pipe
networks, reference lines, breach progression or sediment output) that the engine can
list and read with typed identities and an explicit capability per dataset. This page
describes the generic family inventory and read calls, the stored output-block reader,
time and budget rules, and each qualified family: pipes, environmental and terrain data,
sediment, encroachments, reference points, terrain modifications and 2D mesh corner
elevations. It also states what remains unqualified. The family surface was added in engine
API 8 and extended in API 9 (bitfield attributes) and API 10 (terrain modifications, mesh
corner elevations).

## Inventory and reads

`read_family_inventory(ReadFamily, metadata_max_bytes)` exposes per-dataset source/table/row identities, storage kind, exact shape, attributes, role, location identities and capability. A Supported inventory means the container exists; consult each dataset capability before reading. Present malformed layouts are Corrupt, canonical absence is Unavailable, and unqualified families/layouts are Unsupported with a code. Host I/O failures propagate. `read_family_dataset` resolves an exact family path again, so edited descriptors cannot bypass validation; unrelated result siblings are not decoded.

## Qualified families

Qualified: pipe network/conduit/node/inlet arrays and property tables (6.6 plan/geometry and 7.0 geometry); Location-qualified pipe Base and DSS series; pipe computations and summaries remain unqualified domain layouts with generic literal access; Base/DSS/Adaptive reference-line series with exact Name-axis identities and literal unqualified one-row values; breach progression with Variable_Unit column labels and attributes; Plan Data and run-summary/process/message datasets and group/root attributes. Packed pipe range bounds are checked before reading geometry halves. Geometry remains raw point/part/range/table arrays: no repaired GIS geometry or inferred node coordinates. IDs retain integers; compound payloads retain exact raw bytes plus typed member evidence. Conduit/node identity-table row ordinals remain distinct from the stored ID values in those tables.

## Output blocks

`output_block_inventory` and `read_output_block` read exact stored envelopes under every non-Base unsteady Output Block, including DSS Hydrograph/Profile/Adaptive and Computation Block. These are typed storage envelopes, not a claim that every contained 1D/structure/mesh layout has domain qualification. 1D and structure semantics are covered by the 1D families ([one-d.md](one-d.md)). Numeric reads accept strict rank-matching positive-step slices, preserving dtype and bits; no clipping. Text/compound reads are whole-table. Unknown datatypes remain Unsupported.

## Time axes

Each series checks its own block's stored Time row count. Time must be finite non-decreasing floating-point values with an explicit `Time` attribute equal to `days` (case-insensitive, observed `days`/`Days`); repeated times are preserved without interval or derivative semantics; optional timestamp arrays must have the same shape and decode as fixed text. Raw DSS timestamps are preserved through the output-block reader: some DSS files use second-resolution strings even in the `(ms)` dataset. This reader does not parse/cross-validate calendar strings or invent an epoch/time zone; it does not claim the Base Output civil-time parser accepts those strings. Reference one-row tables without proven time alignment are labelled `one_row_unqualified`, retaining literal stored values without snapshot/extreme semantics or a time group. A summary exposes time row 1 only if that row exists and is explicitly labelled Time; inconsistent extra row labels are retained as source attributes without synthesizing a row.

## Budgets

A dataset that exceeds an inventory budget keeps its header path/shape/storage class and an individual `Unsupported(ResourceLimit)` capability, with attributes/kind omitted when qualification did not finish; sibling datasets remain visible. Header-only fallback never retries the refused payload. Requested reads still return strict budget errors. Group-attribute traversal remains subject to its own per-attribute errors. metadata_max_bytes bounds each consulted dataset/table/attribute; numeric max_bytes bounds selected returned numeric payload before allocation; compound/text max_bytes bounds raw stored table bytes. Compound decoding additionally owns member vectors and raw payload. Identity/name metadata, header parsing, HDF chunk inflation/cache and validation arrays are additional memory, not an RSS ceiling. Each packed-validation array is bounded by metadata_max_bytes. Hierarchy depth/count limits refuse excessive traversals. Inventories can own many bounded records; there is no aggregate inventory byte ceiling.

## Unqualified and out of scope

Explicit unqualified capabilities: canonical reference summaries and the Pipe Network run-summary table require real fixture qualification. The additional readers below cover land-cover/infiltration sidecars and observed preprocessing layouts, sediment, encroachments, terrain structure, Event Conditions and steady output envelopes; unobserved variants remain unqualified. The calibration-region and cell Manning's n readers (see [engine-api.md](engine-api.md)) remain available. Writers, .NET region overrides, rendered RASMapper products, external raster decoding and Python derived analyses are outside the engine. Post Process variants and remaining residual method coverage are not claimed by the unsteady output-block reader.

## Conformance tests

Conformance generation for the `wave3` test (h5py/NumPy are development-only):

```sh
python tests/generate_wave3_oracle.py --fixture-dir /path/to/fixtures --output /path/to/oracle.json
RAS_WAVE3_FIXTURE_DIR=/path/to/fixtures RAS_WAVE3_ORACLE=/path/to/oracle.json cargo test --no-default-features --features catalog,hdf,mesh,results --test wave3 -- --include-ignored --nocapture
```

The manifest records fixture basenames and SHA256 only, so it relocates without edits. The opt-in test fails explicitly if either variable or its data is missing. It independently compares exact dataset sets, expected refusal sets, full numeric payload hashes through 64 MiB, or three separated boxes of up to 1 MiB each for larger arrays, full text, declared compound-member bytes/column names (excluding undefined HDF padding), qualified attribute values, explicit pipe domains/identity tables/row ordinals, block-specific time groups/counts, names, roles and time-row flags. Trivially absent families are labelled per fixture and are not evidence that a present layout is qualified. Small committed excerpts derive from real Austin/Davis data; generators/provenance record the deliberate time-shape and packed-range mutations.

## Additional readers: environmental, terrain, sediment and encroachments

The environmental module qualifies LandCover/Infiltration geometry, legacy parallel class arrays and modern sidecar tables (including v2 HDF), classification polygon bounds, preprocessed physical/stored-cell and face values, and terrain HDF arrays/references. `classification_catalogue` links current-file raster-map names to Variables rows without default parameters, floating ID conversion, or silently selecting duplicate names. It reports ambiguous matches, orphan map names and unmatched Variables rows; virtual dependency composition remains literal dependency attributes, not synthesized raster classifications. Legacy IDs can be byte-wide HDF bitfields; their class codes remain exact unsigned bytes. No-data values are retained.

Since API 9, bitfield *attributes* (e.g. `Connection Profile Hash`, `Property Tables LC Hash`,
`Infiltration Override Table Hash`) are typed `AttributeType::Bitfield { size, order }` with
`MetadataValue::Bitfield(bytes)` holding the exact stored bytes (h5py returns them as
`uint<8*size>` arrays) instead of `Unsupported(encoding)`. Only full-precision, zero-offset,
zero-padding bitfields of width 1/2/4/8 are proven; others stay Unsupported(encoding).
Compound members that are such bitfields decode the same way.

Terrain Mask datasets are also byte-wide HDF bitfields. `StoredValues::Bitfield` returns exact owned bytes and declared shape; it does not coerce them to numeric elevation data. These bitfield reads are whole-array only and check the raw-byte budget before allocation. Other terrain numeric arrays support slices. Terrain group attributes retain source File references, priorities, pyramid-level statistics and projection evidence. External TIFF data is not opened or decoded.

Sediment reads now include root Sediment inputs/options/gradation data, 1D Results/Sediment blocks, 2D Sediment Bed/Transport blocks, sediment time/station-elevation output, static subcell/subface properties and the corresponding mesh centers/areas. Dynamic series check their block-specific Time length. Explicitly qualified initial-elevation/area/length tables keep their single static row; other unmatched one-row sediment tables refuse as unqualified rather than implying extrema or a time series. Encroachment/floodway readers expose exact steady profile variables, stations, plan parameters and identity/profile tables; hydraulic calculations remain Python. EventConditions and SteadyOutput offer bounded stored envelopes and attributes, without adopting the 1D families' hydraulic interpretation.

The independently frozen oracle for these readers (`wave3b` test) hashes full numeric arrays up to 64 MiB (any larger prefix exception is explicit in the manifest), full text and byte bitfields, and all declared compound-member bytes. HDF5 compound padding is undefined by H5Dread and is excluded from equality assertions; the engine still retains its full raw payload. Source shapes and widths remain authoritative: the real Weise Minimum Bed Elevation summary declares only 2x2 elements while its index retains 1,537 aligned chunks. The reader ignores chunks wholly outside the current logical extent, as h5py does, and returns the literal 2x2 values without inventing a mesh-wide summary. Malformed ranks and non-grid offsets retain strict validation.

Qualification remains per object. Canonical reference summaries and pipe run-summary fixtures are still required before those families can be promoted; reference points are qualified by the 6.6 and 7.0.1 fixtures below. One-row reference tables carry no snapshot or extreme semantic label. Complete downstream Python delegation is outside the engine.

## Pipe domains

Pipe domains come only from the stored scalar `Location` attribute and its geometry
count. Dataset names and `Columns` never choose the domain. These are the complete
qualified Location mappings (exact spelling):

| Pipe Location | Domain | Network geometry table |
|---|---|---|
| Cell Water Surface | cell | Cell Polygons Info |
| Face Flow / Face Velocity / Face Water Surface | face | Face Polylines Info |
| Nodes | node | Node Indices |
| Pipe Links | conduit | Conduit Indices |

Missing/unknown/unqualified Location is Unsupported(identity_layout); a known Location
whose column length contradicts its geometry row count is Corrupt. Missing geometry
identity tables and non-integer ID tables remain unqualified. Names may change without
changing a valid Location-derived domain. This replaces the earlier name allowlist.

`FamilyDataset.locations[i].row` is the **position** in the identity table.
`stored_location_ids` separately returns the exact integer NumericArray stored in Node
Indices or Conduit Indices, in column order and with its stored width/signedness.
Davis Node Indices start 121, 85, 119: position zero is not ID zero. Numeric selections
continue to select column positions; callers resolve stored IDs explicitly. Cells/faces
have geometry-row identities and no invented ID array. Other read families leave this
optional field unset. The current real corpus validates all 14 pipe node/conduit arrays.

Davis pipe summaries (32 dataset instances, 16 distinct names) and five computation
datasets do not carry Location. They therefore refuse typed domain qualification even
where their names/counts formerly appeared recognizable; the exact literal arrays
remain readable via read_numeric. Their attributes are retained in refused inventory
entries. The dead Pipe Network run-summary branch and Geometry/Pipe Inlets root remain
unqualified; observed inlets are under Geometry/Pipe Nodes.

## Filter pipelines

Bounded filter pipelines account for DEFLATE wrappers, Fletcher32 checksums and the
scale-offset header/padding. Unknown/custom expansion bounds and LZ4 as an encoded
prefix refuse explicitly; a single built-in LZ4 decoder still enforces its output
size. Custom registry overrides also have no qualified bound. Unlimited generic-reader
filter APIs retain their existing behaviour. No external filter plugin is installed.

## Reference points (6.6 and 7.0.1 real-fixture qualification)

`reference_point_geometry(max_bytes)` returns `Reported<ReferencePointGeometry>` with
exact Attributes compound bytes/member evidence, stored XY Points and named features.
Each feature retains its Attributes-row identity, Points-row ordinal, exact SA/2D name
and stored signed i32 Cell Index. No physical/ghost renumbering, nearest-cell search,
coordinate conversion or invented packed offsets occurs. The observed 2D layout is
qualified; 1D reference-point interpolation layouts remain unqualified. Per-object
stored reads and identity allocations use max_bytes, without an aggregate RSS bound.

`GeometryFamily::ReferencePoints` capability and point layers route through this API.
`geometry_family` is the packed-line/polygon envelope and explicitly directs point
callers to `reference_point_geometry`; it cannot represent a point without inventing
Info/Parts arrays. Other geometry families retain their existing signatures.

`ReadFamily::ReferencePoints` supports Base Output and DSS Hydrograph Output global
Name/Water Surface/Velocity/Direction and per-area Name/Water Surface series. Names
are kept literal: global names include `|area`, per-area names do not. Each exact
Name identity must uniquely match a geometry Name/SA/2D pair. `locations` keep Name
row identities and `geometry_locations` keep the corresponding geometry Attributes
identities in column order, including reordered/subset Name axes. Cells and global
2D Areas attributes must agree with those named identities. The block-specific Time
and Name dimensions are checked; no global/Base Time substitution occurs for DSS.
The legacy plan capability for reference points now reports these qualified reads,
aggregating per-entry failures; layer fields preserve per-dataset capabilities.

`ReadFamily::ReferenceLines` results are linked to `Geometry/Reference Lines/Attributes`
rows the same way, reported separately in `geometry_link` so that a failed link never
changes a dataset's read capability. Inventories report the link; a value read
(`read_family_dataset`) does not prove it, since its outcome cannot change the values. Each literal Name entry (`<name>|<SA-2D>`) must
uniquely match one geometry Name/SA-2D pair; the Name axis's `<name>|Faces` attribute must
equal, in order, the geometry `Internal Faces` of that row; and `2D Areas` ordinals, where
stored, must name the same areas. `geometry_link` is Supported exactly when
`geometry_locations` is populated (in column order, including reordered Name axes);
Unavailable when the plan stores no reference-line geometry; Unsupported(identity_layout)
for an unmatched, duplicate or incomplete identity layout; Corrupt when stored Faces or
area ordinals contradict the named geometry. HEC-RAS 7.0.1 also writes each area's series
under `Unsteady Time Series/2D Flow Areas/<area>/Reference Lines`, with bare `<name>`
entries and no Faces attributes; those link to the geometry row with that Name and the
path's area, with the Faces cross-check made against the same output block's
`Reference Lines/Name` axis (Unsupported(identity_layout) when that axis is absent).
HEC-RAS 7.0.1 does not write one line name in two areas: it renames the second line
(`RL_Shared` becomes `RL_Shared 1`) in the geometry and the results. Reference points report
`geometry_link` Supported whenever their (mandatory) link succeeds; other families report
Unavailable.

Direction is present in the 7.0.1 fixture and absent in 6.6; its absence does not
refuse the remaining point series or invent a derived angle. Direction is a stored degree-angle series (`Units=deg`, role `direction_series`),
not flow, a velocity component or a north-bearing conversion. Stored f32 values,
including zero/dry values, are returned unchanged. In the provided real Base output,
290 samples with speed >0.01 match atan2(Vy,Vx) in degrees within 7.63e-6 degrees;
this supports the observed positive-quadrant convention, not an unobserved wrap or
negative-angle policy. No angle is derived by the reader. Other units refuse instead
of silently converting. Reference-point summaries, other output blocks, unobserved
variables (including Flow) and per-area Velocity/Direction remain Unsupported.

The fixtures come from a one-day plan derived from the public BaldEagleDamBrk example
and computed in HEC-RAS 6.6 and 7.0.1. ras-commander's `GeomReferenceFeatures` initially
wrote IC Point forms, so the computed runs used manually inserted native Reference Point
blocks in the geometry and manually edited output flags. The reference line was added
through the ras-commander API. This qualification uses only immutable copies of those
outputs and performs no model run or flag editing. All four geometry/plan
HDF hashes are verified. The oracle checks 30 complete point payloads (including
geometry compounds by declared member, excluding undefined padding), every point
series value, attribute and identity. Both versions have three points, 165 Base
rows and 1,441 DSS rows, and no point Summary Output.

The same oracle checks all 72 reference-line payloads and their exact variable sets
in both blocks and global/per-area scopes. At top level 7.0.1 has 15 numeric series;
6.6 has six numeric series plus Name (seven datasets), including Water Surface
Conveyance. Per-area sets also differ. The literal negative RL_Mid Flow is preserved
(minimum about -957 cfs); the reader does not reverse a drawn line's flow sign.
AustinOyster.p01 has no Face Flow dataset; its observed flows are Reference Lines/Flow.

```sh
python tests/generate_reference_points_oracle.py --fixture-dir /path/to/refpoint-fixtures --output oracle.json
RAS_REFERENCE_POINTS_ORACLE=oracle.json RAS_REFERENCE_POINTS_FIXTURES=/path/to/refpoint-fixtures cargo test --no-default-features --features catalog,hdf,mesh,results --test reference_points -- --include-ignored --nocapture
```

## Reference results joined to geometry

`reference_line_results(max_bytes)` and `reference_point_results(max_bytes)` (Rust only)
return `Reported<ReferenceLineResults>` / `Reported<ReferencePointResults>`: the exact
geometry (`geometry_family(ReferenceLines)` or `reference_point_geometry`), every joined
result dataset (its `FamilyDataset` descriptor and whole stored `[time, column]` values,
unconverted), one entry per geometry feature in row order with the `(dataset, column)`
pairs linked to it, and `omitted` (path, capability) for datasets not joined.

The join adds no matching of its own. A numeric dataset is joined only when its read
capability and its `geometry_link` are Supported; each column goes to the feature whose
Attributes row its `geometry_locations` identity names. The whole join is refused, with
that status, when a qualified dataset's link is Unsupported, Unavailable or Corrupt, or a
dataset fails with Unsupported(identity_layout) or Corrupt (reference-point series prove
their identities while qualifying); geometry that is absent or refused gives the join the
geometry's status. Datasets the family does not qualify for other reasons (for example
point summaries or unobserved variables, Unsupported(schema)) are listed in `omitted`,
never dropped silently. Name axes are not listed. Values are read only once the whole
join is proven; `max_bytes` bounds each consulted dataset independently.

`tests/reference_results.rs` checks every reference-line and reference-point fixture
(valid, reordered, per-area, and each unprovable layout). On real files,
`tests/reference_results_oracle.py` joins the same series to geometry rows with h5py from
the Name axis alone (global `<name>|<area>`, per-area `<name>` plus the path's area) and
records digests of each row's coordinates and each linked column. On the five corpus plans
with reference geometry and results, all 700 linked columns match; the eBFE 6.x plan's
lines are refused (IdentityLayout: no integer `Internal Faces` columns), as listed in the
test.

```sh
python tests/reference_results_oracle.py --fixture-dir /path/to/reference-plans --output oracle.json
RAS_REFERENCE_RESULTS_ORACLE=oracle.json RAS_REFERENCE_RESULTS_FIXTURES=/path/to/reference-plans cargo test --no-default-features --features catalog,hdf,mesh,results --test reference_results -- --include-ignored --nocapture
```

## Environmental and sediment qualification rules

Environmental qualification requires polygon arrays in an observed nonempty
classification parent namespace; unexpected root polygon arrays remain schema
refusals with generic literal access. Dynamic sediment numeric results require a
qualified time/element orientation and their own Time axis; explicit Time/text/static
metadata are separate envelopes. Scalar/vector dynamic results do not become series.
Floodway Profile Names carry a profile/name axis. Only matching cross-section arrays
claim cross-section placement; other stored metadata remains untyped. The real `wave3`/`wave3b`
qualification harnesses require 9/36 source cases and 7,970/705 payload checks; empty
fixtures/manifests are errors, not successful qualification.

## Terrain modifications (`terrain_modifications`, API 10)

`ReadFamily::TerrainModifications` lists the RASMapper terrain-modification attribute
tables of a terrain HDF: `Modifications/<group>/Attributes` and, when present,
`Modifications/<group>/Control Points/Attributes`. Group attributes (`Type`, `Subtype`,
`Priority`) of `Modifications`, each group and each Control Points group are reported as
typed family groups. Geometry arrays of the same groups (Polyline/Polygon Info, Parts,
Points, Profile, Profile Info/Values, Circles, Piers, Rectangles, Control Points/Points)
are not members: they remain literal generic-reader data. A file without `Modifications` is Unavailable.

A table is qualified only when its group's stored scalar text `Type` lists an observed
layout and the stored compound is exactly that layout: member names and order, packed
offsets with no compound padding, fixed-length ASCII text (any proven padding, any
nonzero width), IEEE little-endian 4-byte floats and unsigned 1-byte integers. A `Type`
with two observed layouts tells them apart by member count; a near miss is refused with
the defect against the layout of the same count.

| group `Type` | role | members | aligned with |
|---|---|---|---|
| `Levee` (Channel and Levee subtypes) | `polyline_modification_attributes` | Name, Elevation Type, Top Elevation, Top Width, Left Slope, Right Slope, Max Reach, Elev Pt Tolerance | `Polyline Info` |
| `Levee` (Channel and Levee subtypes; HEC-RAS 6.4.1-6.6 era terrains and ras-commander `RasTerrainModWriter`) | `polyline_modification_attributes` | System Name, Name, Elevation Type, Top Elevation, Top Width, Left Slope, Right Slope, Max Reach, Transition Percent, Elev Pt Tolerance | `Polyline Info` |
| `Channel` (written only by ras-commander `RasTerrainModWriter.add_channel_modification`; the 10-member layout only) | `polyline_modification_attributes` | System Name, Name, Elevation Type, Top Elevation, Top Width, Left Slope, Right Slope, Max Reach, Transition Percent, Elev Pt Tolerance | `Polyline Info` |
| `Polygon` | `polygon_modification_attributes` | Name, Elevation Value, Elevation Type, Elev Pt Tolerance, Generate Boundary Elevations, Use ShapeFile Z Elevations | `Polygon Info` |
| `Circle` | `circle_modification_attributes` | Name, Elevation Value, Elevation Type | `Circles` |
| `Pier` | `pier_modification_attributes` | Name, Elevation Value, Elevation Type | `Piers` |
| `Rectangle` | `rectangle_modification_attributes` | Name, Elevation Value, Elevation Type | `Rectangles` |
| any of the above, Control Points | `control_point_attributes` | Name, Elevation | `Control Points/Points` |

The aligned table is the dataset's `identity_table`; its row count must equal the
Attributes row count (Corrupt otherwise). A missing or unobserved `Type`, a missing
aligned table, a rank other than one, vlen/UTF-8 text, other widths, big-endian or signed
members, reordered/missing/extra members or compound padding are Unsupported(schema)
(vlen members Unsupported(encoding)). A truncated or damaged payload is Corrupt. Each
table carries its own capability, so one refused table does not hide its siblings.

`read_family_dataset` reads one exact table path whole (slices are refused as
InvalidRequest) and returns `StoredValues::Compound` with the exact stored bytes and
per-member type evidence. Text is not trimmed: h5py's `dataset[()]` bytes follow from the
stored bytes by HDF5's fixed-string conversion (NULLPAD copied, NULLTERM cut at the first
NUL, SPACEPAD trailing spaces removed, the remainder zero-filled), which bindings apply
from the member padding evidence. `metadata_max_bytes` bounds each consulted table and
attribute; `max_bytes` bounds the returned raw table.

Evidence: synthetic fixtures with invented values reproduce the observed stored schema
(`tests/generate_terrain_modification_fixtures.py`, `tests/terrain_modifications.rs`:
16 qualified tables compared byte for byte with stored and h5py bytes, 25 refused defects
including near misses of every multi-layout Type, truncation, path and budget refusals).
The layouts were taken from a header survey of 169 non-distributed terrain HDFs that carry
modification tables: real HEC-RAS 6.1-6.6 derived terrains and ras-commander writer and
notebook output (1,851 Attributes tables, all qualified and read byte-equal to h5py; those
files are not shipped). Nothing else was observed outside unit-test scratch output (whose
invented layouts and vlen `Type` stay refused); any other `Type` or layout stays
Unsupported(schema) until a real file shows it.

## Mesh corner elevations (`mesh_corner_elevations`, API 10)

`ReadFamily::MeshCornerElevations` lists the static ground elevation of every 2D mesh
corner (face point). RASMapper's depth-weighted sloping interpolation uses these terrain
values. They are not results: corner water surfaces, face water surfaces and node
velocities are different data. HEC-RAS stores them in two layouts:

| layout | role | dataset | written by |
|---|---|---|---|
| geometry | `geometry_corner_elevation` | `Geometry/2D Flow Areas/<mesh>/FacePoints Elevation` in a geometry or plan HDF | 2D hydraulic-table preprocessing, from the terrain face-profile endpoints |
| mapping cache | `mapping_cache_corner_elevation` | `Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas/<mesh>/Processed Data/Profile (Horizontal)/FacePoint Elevation` in the result's `PostProcessing.hdf` | RASMapper, when it renders depth-weighted sloping maps |

Note the spellings: `FacePoints Elevation` (geometry) and `FacePoint Elevation` (cache).

An array is qualified only when it is a rank-one little-endian `f4` array with exactly
one value per mesh corner. Values come back exactly as stored, in `FacePoints Coordinate`
row order: NaN payloads, signed zero, infinities, subnormals and sentinels such as -9999
are not altered. Another dtype or byte order is Unsupported(encoding); another rank
(scalar included) is Unsupported(schema); a length that differs from the corner count is
Corrupt. Numeric selections are allowed. A file without either layout is Unavailable.

The corner count of a geometry-layout array is the row count of the same mesh's
`FacePoints Coordinate` (shape `(corners, 2)`), which is its `identity_table`; missing
coordinates or coordinates of another width are Corrupt. A cache file stores no
coordinates, so its inventory row is Unsupported(identity_layout): only the typed reader,
given the corner count of the plan or geometry HDF the cache belongs to, qualifies it.

`ReadOnlyHdfSession::mesh_corner_elevations(mesh, layout, expected_corners, max_bytes)`
reads one mesh in one layout and returns `Reported<CornerElevations>`: the values, the
source dataset (file identity, path, shape, every attribute), the corner count and where
that count came from (`CornerCountSource::Coordinates` or `Caller`). An absent dataset or
mesh is Unavailable; a refused layout is Unsupported; a disagreement with
`expected_corners` or with the stored coordinates is Corrupt. The Python extension
exposes it as `mesh_corner_elevations(path, mesh, layout, expected_corners=None,
max_bytes=...)`. The engine never chooses between the two layouts, compares them,
interpolates or generates elevations: a caller that holds both decides (ras-commander's
`HdfMesh.get_mesh_corner_elevations` does).

Evidence: synthetic fixtures with invented values reproduce the observed stored schema
(`tests/generate_corner_elevation_fixtures.py`, `tests/corner_elevations.rs`: qualified
arrays compared byte for byte with h5py bytes, eight refused defects, cache count rules,
absent data, budgets and path validation). The geometry layout was taken from a survey
of the release fixture corpus: 41 mesh arrays in 35 HEC-RAS 7.0 and 7.0.1 geometry and
plan HDFs, all rank-one gzip-chunked little-endian `f4` with a `Can Plot = False`
attribute and one value per corner, all read natively bit-identical to h5py. None of the
HEC-RAS 5.x and 6.x files in the corpus has the dataset. The cache layout was taken from
two `PostProcessing.hdf` files that native HEC-RAS 7.0.1 RASMapper wrote for
depth-weighted sloping maps (rank-one gzip-chunked `<f4` with `Terrain Filename` and
`Terrain File Date` attributes, no geometry), both read bit-identical to h5py and to the
plan's `FacePoints Elevation`. Check further files with `tests/corner_elevation_oracle.py`
and the ignored test `real_files_match_the_h5py_oracle`; those files are not shipped.
