# Engine API contract

This page is the reference contract for the Rust engine in the crate `ras-commander-fast-hdf`
(`ras_commander_fast_hdf` in Rust source; formerly `ras-commander-rust`), currently at engine
`API_VERSION` 10. It describes
what the engine reads from HEC-RAS project text and HDF files, how every result reports
whether it is supported, unavailable, unsupported or corrupt, how byte budgets and errors
work, and the exact rules for each reader. Families with their own detailed pages are
linked from here: [read families](read-families.md), [mesh results](mesh-results.md),
[1D families](one-d.md) and [session caching](perf-session.md). The
[API version history](#api-version-history) at the end lists what each `API_VERSION` added.
Examples are not a promise of an unimplemented API; `api::ENTRYPOINTS` and
`capabilities()` are the machine-readable list of what exists.

## Scope

Domain surfaces:

- **Project/plan catalogue**: titles, current and listed plans, linked
  geometry/steady/unsteady files, declared simulation times and intervals, computed-result
  availability and source diagnostics. Read-only discovery; unknown fields and original
  bytes remain available.
- **HDF source/session**: read-only path-based access, typed numeric capabilities,
  selections, batches and metadata. Session lifetime, copies and caches, and unavailable
  types are explicit. Byte/range sources would be a later generic-reader extension.
- **Geometry**: named mesh topology, composite mesh/cell identities, physical/ghost
  distinctions, packed coordinates/offsets/IDs, and strict versus explicit-quarantine
  diagnostics. No hidden snapping, synthesized polygons or source repair. Packed ring order
  is a contract (API 10): clockwise from the first listed face, consecutive duplicates
  dropped, cells with fewer than three faces omitted.
- **Results**: selected arrays and bounded temporal reduction with explicit orientation,
  missing/dry/threshold/tie semantics, physical counts and stored row indices; timestamps
  and reported units via `hdf::time` and `hdf::units` (conversions are out of scope). 2D,
  1D, steady, reference and pipe breadth is reported per capability.
- **CRS**: raw WKT and domain source precedence/evidence, unit/axis information and optional
  verified authority. An unknown or conflicting CRS is never a guessed EPSG code.

Boundaries:

- No PyO3, NumPy/pandas, GeoDataFrame or Python objects in public Rust signatures. Typed
  errors distinguish unsupported features, malformed storage/topology, missing data and
  I/O. Nothing routes through Python to make native conformance pass. Integer precision
  and native values are preserved; any result-specific `f32` narrowing is named and
  documented.
- The Python extension (`python/`) translates these results and preserves ras-commander's
  Python signatures, defaults, logging, error policy and return types; ras-commander may
  keep efficient h5py/NumPy/Shapely/pyproj paths. Other consumers translate domain values
  into their own schema, layer, error and I/O abstractions. Cargo features control
  dependency inclusion, not symbol imports alone.
- Fluvial/pluvial analysis, text editing workflows, exports/PMTiles, HEC-RAS execution and
  GUI/remote orchestration are not engine scope (some of these live in the Python
  extension). Reusable byte parsing needed for read-only project/plan/geometry discovery is
  in scope. New analysis or format features require a deliberate scope decision.

## Features

`default = catalog` (std only). `hdf` enables the generic read, metadata and 1D readers;
`mesh` and `results` each imply `hdf`. CRS lexical types are unconditional, with
catalogue/HDF helpers feature-gated. The crate declares `rust-version = "1.88"`
(`as_chunks`).

Existing reduction maximum/offset buffers are `f32`; `f64` inputs narrow only in that
explicitly result-specific API. Numeric selected arrays preserve dtype. Public structs and
enums are `#[non_exhaustive]`; `Topology::new`, `Slice::new`, `ReductionRequest::new` (with
fluent options and `Default`) and `Reduction::new` construct them.

## Capabilities and status vocabulary

`Capability` is the public outcome shape: `Supported`, `Unavailable{reason}`,
`Unsupported{code, reason}`, `Corrupt{reason}`, `Delegated{reason}`. Every evidence value
answers with a single `Capability`; no wrapper can hold a contradictory state. Inspect
typed codes; never parse reason or display strings.

| `as_str` | Meaning | Carries |
|---|---|---|
| `supported` | Proven and usable (CRS: lexically readable, not a validated CRS) | nothing |
| `unavailable` | The object or attribute is canonically absent | reason |
| `unsupported` | Present but unproven or over a limit | `UnsupportedCode`, reason |
| `corrupt` | Present but damaged or contradictory | reason |
| `delegated` | Validation is left to an existing per-object API | reason |

`UnsupportedCode` (non-exhaustive): `encoding` (unproven dtype, text or datatype envelope),
`schema` (structural gap), `identity_layout` (unproven identity layout), `resource_limit`
(count, depth or byte cap), `not_implemented` (qualified but unimplemented).

- `NumericCapability` and `VariableDescriptor` contain a capability; `ResultAvailability`
  contains a capability plus variables. `xs1d::FamilyStatus` aliases `Capability`.
- `Reported<T>` (private fields; `value()`, `raw_bytes()`, `capability()`, `is_supported()`,
  `is_unavailable()`; constructors `supported`, `unavailable`, `unsupported(code, reason)`,
  `corrupt`, `with_raw_bytes`) is the evidence wrapper. `XsField<T>`, `UnitsEvidence` (also
  `DatasetUnits.units`, `VariableUnits.units`, `UnitSystemEvidence.value`), `ColumnLabels`
  (value `ColumnLabelSet{source, labels}`) and `NodePointer` are type aliases of it.
  Supported iff a value is present; Unsupported and Corrupt keep the stored `raw_bytes`.
- `MetadataValue::{Unsupported{code, reason}, Corrupt{reason}}`,
  `AttributeType::Unsupported{code, reason}` and `EvidenceStatus::{Unsupported{code, reason},
  Corrupt{reason}}` derive their capability with `capability()`. `EvidenceStatus` maps
  Unknown to Unavailable, Unvalidated to Supported (lexically readable only, not a validated
  CRS), Invalid to Corrupt and Conflict to Unsupported(Schema). `CrsEvidence`, `CrsReport`,
  `DatasetUnits`, `VariableUnits` and `UnitSystemEvidence` provide `capability()`.
- `EvidenceStatus` outer variants are authoritative: engine-produced Unsupported/Corrupt
  payloads match their outer variant; caller-constructed contradictory `Capability`
  payloads must not be trusted as evidence validity. This is a documented contract, not a
  type-level restriction.
- Catalogue `Availability` and `PlanResultMode::Unavailable` are catalogue/plan vocabulary
  rather than evidence status and keep their own types.
- `NumericCapability.unsupported_reason` remains as a deprecated field for older callers;
  use `capability.reason()`.

### Stable spellings

`as_str()` returns the stable snake_case spelling (never `Debug`) for `ErrorKind`,
`Capability`, `UnsupportedCode`, `ResultFamily` (`mesh_2d`, `cross_sections_unsteady`,
`steady_profiles`, `structures`, `pipes`, `reference_lines`, `reference_points`,
`boundary_conditions`, `calibration_regions`), `StructureFamilyKind`, `PlanResultMode`,
`ResultMode`, `UnitsFamily`, `UnitSystemKind`, `NumericKind` (`f32`, `i64`, ...) and
`ByteOrder`. `hdf::ResultMode {Steady, Unsteady}` replaces `MeshResultMode` and
`XsResultKind` (deprecated type aliases). `PlanResultMode` (plan-level, with
Mixed/Unavailable), `UnitsFamily` (variable family) and the catalogue `FlowKind` carry
different semantics and stay separate.

### Datatypes

`hdf::AttributeType` describes proven `Int{signed, bytes, order}`, `Float{bytes, order}`,
`FixedString{charset, padding, width}`, `Bitfield{size, order}` (API 9), `Compound` or
`Unsupported{code, reason}`. `TypedAttribute` and `CompoundColumn` expose `attribute_type`
plus `datatype_label`; the label is display-only. Legacy `Attribute` also exposes the
fields, but its type is always Unsupported because that high-level generic path discards
the raw envelope; use `typed_group_attributes` for proof. `EncodingDescriptor.class` is a
typed `TypeClass` (Integer/Float/FixedString/Compound/Other); `TypeClass::code` returns the
raw nibble, `TypeClass::Other` carries an opaque `OtherTypeClass` (construct with
`TypeClass::from_code`, use wildcard matching), and the deprecated
`EncodingDescriptor::class_id` helps migration. No datatype proof derives from `Debug`
strings. Selected numeric values remain exact.

## Errors

`Error.code: Option<UnsupportedCode>` carries the reason an `Unsupported` (or a
budget-limited `InvalidRequest`) error is unsupported. Producers set it with
`Error::unsupported(code, msg)` / `with_code`; no consumer defaults it. Caps are
`ResourceLimit`, unproven dtype/text are `Encoding`, unproven identity layouts are
`IdentityLayout`, remaining structural gaps are `Schema`, unimplemented features
`NotImplemented`.

Missing files map to `FileNotFound`, missing HDF objects to `NotFound`, bad input to
`InvalidRequest`, unproven features to `Unsupported`, damaged input to `Corrupt`, pool-build
failure to `Internal`. `TypeMismatch`/`SliceOutOfBounds` are `InvalidRequest`, and
unresolved external raw data files `FileNotFound`. Error classification uses typed
variants plus a documented allow-list of reader `Other` messages in `encoding.rs`; unknown
messages remain Corrupt. `InvalidData` and wrapped failed-to-load shared-message index
errors remain Corrupt, not guessed Unsupported. `CatalogError::new` supports external
construction of the non-exhaustive error struct; `CatalogErrorKind` and `Availability`
matches require wildcard arms.

`CivilTimestamp::new` with an out-of-range field and `crs::relative_projection` with an
absolute or escaping reference are `InvalidRequest` (caller input). Readers that decode
stored values (`parse_hec_stamp`, `parse_hec_ms_stamp`,
`CivilTimestamp::from_ms_since_epoch`, time-axis reads) report impossible stored values as
Corrupt.

One crate-internal mapping (`capability::error_to_capability`) turns a failed family check
into a `Capability` for xs1d, structures and mesh output blocks:

| `ErrorKind::as_str` | Raised for | Becomes in a capability report |
|---|---|---|
| `io`, `internal`, `file_not_found` | host I/O, pool build, unresolved external file | always propagates as `Err` |
| `not_found` | missing HDF object or attribute | `Unavailable`/skipped when absence is canonical (`absent_ok`); `Corrupt` when a parent promised it |
| `unsupported` | unproven feature (`Error.code` says why) | `Unsupported{code}` |
| `invalid_request` | caller input (selection, byte budget, timestamp range, projection path) | `Unsupported{resource_limit}` when a budget; otherwise returned to the caller |
| `corrupt`, `structural_topology`, `invalid_geometry` | damaged storage or topology | `Corrupt` |

Canonical absence of an optional object is decided before mapping by `absent_ok`
(`NotFound` becomes `None`): a missing optional group is Unavailable or skipped, never
Corrupt, and other failures never become absence. Capability reasons are bare messages
(no `Kind:` prefix). Python exception mapping is defined by the extension; see
[extension-contract.md](extension-contract.md).

## Byte budgets

In every metadata call, `max_bytes` bounds the raw stored size of any single dataset,
table or attribute the call consults; exceeding it is `Unsupported(ResourceLimit)` in
capability reports and `InvalidRequest` carrying `code = ResourceLimit` from direct reads.
Only an `InvalidRequest` that already carries `ResourceLimit` becomes a status; every other
`InvalidRequest` propagates. Result selections (`read_cross_section_result`,
`read_structure_series`) bound the bytes they return with their own `max_bytes`.
`hdf::DEFAULT_MAX_BYTES` (64 MiB) is the recommended metadata value; pass the same number to
a capability report and to the matching reads to get matching verdicts. One exception:
steady profile names and packed result-identity string tables are never read above
`xs1d::MAX_NAME_TABLE_BYTES` (8 MiB), whatever `max_bytes` is.

`mesh_inventory(max_bytes)`, `mesh_name_diagnostics(max_bytes)`,
`plan_capabilities(max_bytes)`, `steady_profile_names(max_bytes)` and
`result_units(max_bytes)` take an explicit budget. `plan_capabilities` passes its budget to
the XS geometry, identity tables, profile names, time axis and structure inventories.
`read_cross_section_result(kind, variable, rows, sections, metadata_max_bytes, max_bytes)`
uses `metadata_max_bytes` for the same geometry, identity and profile-name validation as
`plan_capabilities`, and `max_bytes` for returned bytes; `read_structure_series` validates
no budgeted metadata, so it has only the returned-bytes `max_bytes`. Low-level
`dataset_units`, `dataset_units_attribute` and `unit_system_evidence` use
`DEFAULT_MAX_BYTES`; the deprecated `initial_2d_inventory()` and `dataset_unit_evidence`
aliases do too. An attribute over budget is reported `Unsupported(ResourceLimit)` without its
bytes. `max_bytes` bounds selected result bytes before allocation; HDF whole-chunk inflation
is a separate storage cost.

### Whole-read budgets (API 10)

Whole-read entry points have a budget. Without one, a 1.4 KB file declaring 2^34 values
could make the host allocate `declared dims x element size` and abort.

- `hdf::DEFAULT_READ_MAX_BYTES` (2 GiB of decoded values) is applied by the existing
  `read_numeric(path, selection)`, `read_numeric_batch(paths)` and `read_topology(mesh)`.
  Their signatures are unchanged.
- Explicit-budget entry points: `read_numeric_bounded(path, selection, max_bytes)`,
  `read_numeric_batch_bounded(paths, max_bytes)`, `read_topology_bounded(mesh, max_bytes)`.
- The budget covers the values returned. For a selection, that is the selected elements,
  not the declared dataset.
- A batch and a topology share one budget across their arrays. Every array's size is
  checked from metadata before any array is read.
- Over-budget and overflowing reads are refused **before allocation** with
  `ErrorKind::Unsupported` and `code = ResourceLimit`. This differs from the caller-budget
  `InvalidRequest` + `ResourceLimit` of the result selections above, because the default
  applies without a caller request. Inspect `code`, which is `ResourceLimit` either way;
  `Capability::from_error` reports both as `Unsupported(resource_limit)`.

Legacy whole reads above 2 GiB fail with `ResourceLimit` instead of allocating. Callers that
need more pass a larger explicit budget.

## Identity

`hdf::Identity {source: PathBuf, table: String, row: u64}` locates a described object by
file, attribute table and row. It is embedded as `CrossSection.id` (table
`Geometry/Cross Sections/Attributes`), `GeometryIdentity.id` (the family's geometry
attribute table) and `MeshDescriptor.id` (table `Geometry/2D Flow Areas/Attributes`).
Domain names stay domain-specific (`river/reach/station`,
`connection/upstream/downstream`, `mesh_name`). `NamedMesh` (a topology read by name, with
no attribute row) and time row indices do not carry an `Identity`.

## Project catalogue containment

Catalogue declarations are project-relative and lexically bounded before probing. Rooted,
drive-relative, absolute, device and UNC declarations are retained as `OutsideProject`
without resolving, reading or statting their targets. This intentionally includes absolute
declarations that happen to name a path inside the root. Malformed or unreadable links are
`InvalidReference`. `SourceLink.diagnostic` and `ProjectDiagnostic::LinkError` retain the
per-link failure while other listed plans remain readable. Dependencies of
catalogue-loaded plans use the project root, so plan-relative `../` is allowed inside that
boundary. Standalone `Plan::open`/`from_document` uses the plan parent as its boundary.
Malformed flow references use `FlowKind::Unknown`. Derived geometry and results HDF
resolution errors remain `SourceLink` `InvalidReference` diagnostics instead of aborting
the plan.

Containment is lexical only: symlinks and junctions are not resolved or sandboxed. On
Windows, containment and case-adjusted resolution compare lexical components with an
ASCII case-insensitive policy; other hosts compare case-sensitively. Only ASCII case
differences are accepted: differing non-ASCII code points never alias and fail closed, with
no filesystem probing and no network probe. Exact non-ASCII names are supported. No Unicode
normalization, uppercase table or lossy OS-string replacement is used (a Unicode uppercase
policy would not reproduce NTFS identity and could admit sibling-directory escapes; tests
reject dotless-i, long-s, final-sigma, micro-sign, U+1C90 and titlecase-digraph aliases).
This lexical policy does not reproduce every filesystem configuration.

## CRS and metadata evidence

`CrsEvidence.raw_bytes` retains available HDF attribute payloads. `EvidenceStatus::Corrupt`
preserves corrupt projection outcomes. Projection byte and decoded-text limits are both
1 MiB. Metadata is reported per attribute (Corrupt/Unsupported with raw payload);
non-text `Projection`/`Units` produce explicit evidence; project files are read with
bounded UTF-8/BOM/Latin-1 decoding.

`raw_crs_evidence` does not decode `Units System` itself: `CrsReport.hydraulic_units` is the
text of `unit_system_evidence("/")` when supported, and an unsupported or corrupt attribute
yields one `CrsEvidence` in `hydraulic_units_evidence` whose status and `raw_bytes` come from
that same evidence (a duplicated attribute is Corrupt, as in `unit_system_evidence`).
Absence adds nothing. Semantic CRS validation and authority/unit inference are not
implemented.

The two small committed malformed attribute fixtures run in the default tests; their
generator requires h5py/NumPy only and is not needed at test runtime.

## Mesh inventory and output blocks

`mesh_inventory()` is the canonical descriptor-list method; the deprecated
`initial_2d_inventory()` is an exact delegating alias. `mesh_names()` is available with
`hdf` alone. Before reading centres, the inventory cross-checks Attributes `Name` entries
against group names. Any missing, extra, duplicate or invalid name makes the inventory
Corrupt; `mesh_name_diagnostics()` returns every typed issue and name so callers can inspect
disagreements without an empty success. Names-only layouts remain unsupported. Case is exact
for mesh identity; the catalogue case policy is not applied to HDF object names. Empty
Attributes plus zero mesh groups is Unsupported, never an empty inventory success. Mesh
`Cell Count` may be a proven unsigned column.

`MeshDescriptor.results` keeps the unsteady Base Output compatibility view. `output_blocks`
contains `MeshOutputBlock {mode, block_name, dataset_root, results}` with mode
Steady/Unsteady. All present groups and datasets below each family's Output Blocks are
inspected; missing families have explicit Unavailable entries. Unsteady families without
output blocks (including `Results/Unsteady` present without `Output/Output Blocks`) are
Unavailable; present Steady families lacking a qualified layout are Unsupported(Schema).
Wrong-type family or Output Blocks nodes fail the whole inventory as Corrupt; wrong-type
individual block nodes yield block-level Corrupt. Canonical Unsteady Time Series and
Steady Profiles / 2D Flow Areas paths are discovered; other layout variants are explicit
unsupported states, never assumed absent. Supported means successful dataset discovery;
inspect each variable capability, not compute success, time semantics or hydraulic
completeness. Legacy `results` covers only Base Output. A per-inventory group cache reuses
`/Results` traversal across meshes; there is no global or cross-session cache here.

`TemporalReduction.chunk_shape` is the source storage dimensions as `Option<Vec<u64>>`;
`None` means no chunk layout (compact, contiguous or virtual storage). It is captured
during the existing capability check, so bindings can use it directly and derive counts
from `values.maximum.len()`/`values.rows` without a second `numeric_capability` lookup.

`Accumulator` implements `Debug`; `ReductionRequest` `PartialEq` follows IEEE float
equality (requests containing NaN compare unequal even to their clones). Compound member
offsets, coordinate counts and accumulator row counts use checked conversions and
addition.

## Topology assembly and thread pools

`Topology::assemble_with_session(policy, &session)` borrows the session-owned pool and
honours `parallel = false` via serial iteration. No pool is built per assembly.
Standalone `assemble(policy, threads)` keeps its own bounded pool for callers without a
session. There is no global Rayon configuration and no ownership transfer to `Topology`.
`session.thread_count()` exposes the pool's worker count. Output is owned; source and pool
can be dropped afterwards. Assembly validates once per public call. Read callers should
avoid multiplying outer worker concurrency by unnecessary session worker counts. Pools are
shared per worker count across sessions; see [perf-session.md](perf-session.md).

### Ring order (API 10)

Ring order is an explicit guarantee, and it reproduces ras-commander's Python
`get_mesh_cell_polygons` (GEOS polygonize) **byte for byte**:

- The exterior ring runs **clockwise**.
- It starts at the start point of the cell's **first listed face**
  (`Cells Face and Orientation Values[start, 0]`), traversed in the cell's clockwise
  direction: B to A when that face's orientation value is `+1` (the cell is left of A to B),
  A to B when it is `-1`.
- The remaining faces are walked around the cell, each inserting its `Faces Perimeter
  Values` in traversal order (reversed when the face is walked B to A).
- **Consecutive duplicate coordinates are dropped**, then the ring is closed. Validation sees
  exactly the emitted ring.
- A cell with **fewer than three listed faces** is omitted with `InsufficientFaces`
  (`ZeroFaces` for none), as Python reports `boundary_only/insufficient_faces`. A face with
  identical endpoints is a fatal structural error at any face count.
- Equivalently: the reverse of HEC-RAS's stored counter-clockwise corner order, rotated so
  the first listed face comes first.
- Only an orientation of `+1` or `-1` selects the start. Any other value, or a missing
  column, starts at A; the walked ring is tested with robust `orient2d` and rebuilt from B
  of the first face when it is counter-clockwise, so the ring is clockwise whatever the
  stored column says.

Compared with API 9, about 4-21% of rings (the cells left of their first face) are reversed,
rings with repeated coordinates lose the repeat,
and two-face cells with curved perimeters are omitted. Bindings should require API >= 10 to
get results identical to the Python path.

Evidence: the opt-in `tests/ring_order_oracle.rs` compares native rings with the Python
reference (`tests/generate_ring_order_reference.py`) on real fixtures: 1,965,815 rings across
BaldEagleDamBrk p07 (HEC-RAS 6.6), DavisStormSystem p02 (7.0.1), the FEMA eBFE-derived
AustinOyster p01 (6.6) and the FEMA eBFE AustinOyster p03 (5.0.7), with 0 mismatches.
`tests/verify_ring_contract.py` independently recomputes every ring from the HDF arrays with
NumPy, requires `shapely.is_ccw` to be False for every ring, and checks equal cell sets,
omissions and diagnostics.

## Face lines and perimeters (additive within API 10)

`Topology::face_lines()` returns every face as a polyline in a `PackedLines`
(`coordinates`, `offsets`, `face_ids`; line `i` is `coordinates[offsets[i]..offsets[i + 1]]`),
in stored face order. Each line is the face's start point (`FacePoints Coordinate` row
`Faces FacePoint Indexes[f, 0]`), then its `Faces Perimeter Values[start..start + count]` rows in
stored order, then its end point: exactly ras-commander's Python `HdfMesh.get_mesh_cell_faces`.
No vertex is dropped, merged or reordered, and no face is skipped. The topology is validated
first with the same structural checks as `assemble` (StructuralTopology on failure).

`read_mesh_perimeter(mesh, max_bytes)` reads `Geometry/2D Flow Areas/<mesh>/Perimeter` as
ras-commander's `HdfMesh.get_mesh_areas` does for an area that stores it (`Polygon(rows)`): the
stored rows in stored order (`f32` widened exactly), closed by repeating the first point when
the last differs. An absent dataset is NotFound; the Python collection-level
`Polygon Info/Parts/Points` fallback is not implemented. A dataset that is not `(rows, 2)`
floats, holds a nonfinite value, or gives fewer than four ring coordinates after closing is
Corrupt. Nothing is reoriented, simplified or validated as a simple polygon.

`read_mesh_face_lines(mesh, max_bytes)` is `face_lines` for a named mesh that reads only the
four face tables (`FacePoints Coordinate`, `Faces FacePoint Indexes`, `Faces Perimeter Info`,
`Faces Perimeter Values`). Each must be `(rows, 2)` (Unsupported otherwise); the remaining
checks are `face_lines`' (StructuralTopology). The packed output, 16 bytes per coordinate and
per face, must fit within `max_bytes` (Unsupported, `resource_limit`). Cell tables are not read
or validated. Python: `HdfSession.read_mesh_face_lines` (see [sessions.md](sessions.md#mesh-face-lines)).

Evidence: the opt-in `real_face_lines_and_perimeters_match_ras_commander` test
(`RAS_NATIVE_FIXTURES`, `RAS_FACE_PERIMETER_ORACLE` from
`tests/fixtures/face_perimeter_oracle.py`, which calls ras-commander with
`RAS_COMMANDER_FAST_HDF=h5py`) matched every face line and perimeter ring bit for bit on 20
public HEC example files (BaldEagleDamBrk g01–g13 with Attributes tables, Chippewa_2D g01/p02,
DavisStormSystem g02/p02, Muncie g02–g05 and p02/p04/p05; about 470,000 faces). HEC-RAS 5.0 files
without `Geometry/2D Flow Areas/Attributes` (BaldEagleDamBrk p07/p08/p19, g12) have no Python
reference, because ras-commander's h5py path reads no areas there; the engine reads them by group.

## Exact mesh geometry

`mesh_geometry(area, metadata_max_bytes)` returns the mesh identity and physical, stored and
ghost counts plus per-array capabilities for the `MeshGeometryField` variants (including
Faces Minimum Elevation, Cells Center Manning's n and FacePoints Is Perimeter).
`read_mesh_geometry(area, field, rows, metadata_max_bytes, max_bytes)` reads exact stored
axis-zero rows (optional strided `Slice`), preserving all integer widths and `f32`/`f64`.
Ghost rows are retained; no physical-cell mapping is synthesized. Invalid slices and
over-budget reads fail before allocation. Shapes and cell row counts are checked; absent
arrays are Unavailable, unproven encodings Unsupported, malformed shapes Corrupt.
Info/Values property datasets are exposed separately and exactly, including stored
(start, count) indexes and all value columns; no interpolation or recomputation occurs.
These raw arrays do not certify offset/topology validity. `read_mesh_geometry_cached`
reuses a previous descriptor from the same immutable source.

`mesh_geometry_tables(max_bytes)` reads the area Attributes compound (decoded scalar
columns plus exact raw bytes in `ExactCompound`) and the numeric Cell Info array.
`ExactCompound.table` uses the established metadata widening for `f32` columns; use its
`raw_bytes` and column type/offset for exact stored bits, including NaN payloads. Exact
compounds decode and retain one raw read. Each consulted table is bounded separately.
Observed 5.0.7-7.0 Cell Info is numeric, not compound. The read-only h5py oracle is
`scripts/oracle_wave8_layers.py`; run the opt-in `wave8_layers` test with
`RAS_WAVE8_LAYERS_ORACLE` pointing to its JSON manifest.

## Geometry families

`geometry_family(family, max_bytes)` returns `Reported<GeometryFamilyData>` for boundary
condition lines, 2D break lines, refinement polygons, reference lines and land-cover
calibration polygons. Every feature has its Attributes-row `Identity` and exact Name;
`GeometryFeature.name_capability` isolates blank names without discarding geometry. Info,
Parts and Points retain stored numeric types. Feature parts are validated ranges into the
shared points buffer (Parts offsets are relative to each feature's Info range).
External/Internal Faces, Internal Cells and Calibration Table are exact compounds.
`GeometryFamilyData.terrain_profiles` exposes exact Reference Lines Terrain Profiles
Info/Values where present. `geometry_family_capability` isolates unsupported/corrupt
storage without hiding I/O. Each source dataset is bounded separately; total output may
exceed `max_bytes`. Reference points use `reference_point_geometry` and its dedicated
point-layer dispatch (see [read-families.md](read-families.md)). A land-cover group with no
calibration polygons is Unavailable for this family. These geometry readers do not promote
unqualified result time-series groups in `plan_capabilities`.

## Mesh identity comparison

`compare_mesh_identity(other, area, metadata_max_bytes, max_bytes)` compares stored Cells
Center Coordinate, Faces Cell Indexes, and Faces NormalUnitVector and Length, in that
order. It returns both mesh identities and the first shape, numeric-kind or bitwise-value
mismatch, with stored row/column for values. Float NaN payloads and signed zeros are
significant. No mismatch certifies all three complete arrays equal; missing, unsupported
or corrupt arrays are errors. Comparison streams at most 65,536 rows per chunk;
`max_bytes` bounds each input chunk, with two chunks retained. It compares one exact area
name in both files and never infers an area-name mapping.

## Layer catalogue

`layer_inventory(max_bytes)` returns `LayerDescriptor` values with family, area/name,
optional `Identity`, element domain, geometry type, row count, fields and `Capability`. It
includes every canonical mesh geometry field, each mesh output block and its stored
variable paths, six named geometry families, XS metadata when present, all nine plan
result-family reports, qualified structure groups and series, and the 1D layers described
in [one-d.md](one-d.md). The catalogue uses the qualified `mesh_results` inventories for
per-variable `mesh_summary`, `mesh_series` and `mesh_boundary_series` entries, plus
`run_summary` groups; `mesh_result` and `run_summary` carry the underlying exact
descriptors (units, labels and raw attributes included).

- Unknown row counts stay `None`. `LayerField.row_count` records each variable
  independently; result-layer `row_count` is unknown when aggregating variables.
- Mesh topology is labelled topology, not a repaired polygon. Perimeters have area domain
  and one polygon row. Root Cell Points and Polygon Info/Parts/Points are catalogued with
  explicit capabilities as exact packed tables.
- Result time-series descriptors use geometry type `none` and preserve capability codes;
  qualified geometry does not certify reference-line result-series support. Structure
  series descriptors retain exact group names and geometry identities.
- Empty output blocks are Unavailable. Host errors propagate; non-host failures, including
  mesh or plan inventory byte budgets, remain per-layer capabilities, so
  `layer_inventory(0)` still succeeds. Catalogue mesh probes reuse inventory descriptors.
- `LayerDescriptor` derives `PartialEq` (not `Eq`), because typed raw metadata can contain
  IEEE floats. Consumers must inspect the per-field and per-feature capabilities.

## Mesh results

See [mesh-results.md](mesh-results.md) for the stored mesh result entry points, exact-value
oracles, sparse working-byte budgets and remaining capabilities.

`mesh_boundary_links(area, max_bytes)` reports exact Name + SA-2D geometry associations and
exact stored series names, matched geometry identity/series paths and both unmatched sets.
No trimming, suffix removal or fuzzy matching. Independent side capabilities qualify the
unmatched sets; unavailable sides do not certify completeness. Duplicated names or unproven
area association refuse linking.

## Cross sections and plan capabilities

`hdf::xs1d` adds non-exhaustive types and methods on `ReadOnlyHdfSession`. The broader 1D
surface (geometry tables, lines, XS result variables, 1D Summary Output, 1D structures) is
in [one-d.md](one-d.md).

- `cross_sections(max_bytes)` reads the proven scalar compound at
  `Geometry/Cross Sections/Attributes`. Source path plus row index and exact River/Reach/RS
  preserve identity, including repeated station labels across reaches. Name and bank
  stations are `XsField<T>` (`Reported<T>`); a corrupt XS column is Corrupt. Required
  identity fields must be nonempty proven fixed text. The descriptor limit is 100000
  sections in addition to the caller's raw-byte limit. Stored `f32` bank stations widen
  exactly to `f64`, preserving NaN-ness (not every NaN payload); this is explicit metadata
  widening.
- `is_steady_plan()` checks `Results/Steady` group existence without hiding I/O or
  corruption, and rejects a dataset at the group path (unlike Python's membership-only
  check). `plan_result_mode()` distinguishes steady, unsteady, mixed and absent. Neither
  certifies a successful complete compute.
- `steady_profile_names(max_bytes)` proves fixed-string encoding and rejects empty names,
  unsupported vlen/reference encodings and payloads over 8 MiB. Profile names must be
  nonempty and unique after trimming. Strings retain source spelling and padding
  semantics.
- `read_cross_section_result(kind, variable, rows, sections, metadata_max_bytes,
  max_bytes)` selects Water Surface or Flow from the canonical steady/unsteady output
  groups, validates `(profile/time, XS)` orientation against geometry count and steady
  profile count, and returns an owned `NumericArray` preserving `f32` or `f64`. No narrowing
  or transpose occurs. Rows are stored indices, not interpreted dates. Missing data is
  NotFound, an invalid selection InvalidRequest, a malformed shape Corrupt and an
  unproven/wrong dtype Unsupported. For unsteady results the stored `Time` row count must
  equal the result row count (Corrupt otherwise). Reads inherit session thread, lifetime and
  immutable-source rules.
- `plan_capabilities(max_bytes)` reports nine families (Mesh2D, CrossSectionsUnsteady,
  SteadyProfiles, Structures, Pipes, ReferenceLines, ReferencePoints, BoundaryConditions,
  CalibrationRegions) with a capability plus validated XS dataset paths. XS availability can
  include only one of WSE/Flow; inspect the paths. Unsupported takes precedence over
  partial availability when a present variable is unproven. Missing canonical numeric paths
  are Unavailable; present paths with absent dependencies or wrong object types produce
  Corrupt family status. A family may be Unsupported for bounded geometry/profile
  limitations without aborting other families. Reasons accumulate; geometry/profile
  decoding is cached within a report. Malformed storage errors propagate and never become
  absence.
  - Mesh2D is Delegated to the per-mesh readers, whose behaviour is unchanged.
  - Pipes, ReferenceLines, BoundaryConditions and CalibrationRegions remain presence-only
    aggregates here; use the qualified geometry and `ReadFamily` readers.
  - ReferencePoints delegates to the qualified Base/DSS point inventory and aggregates its
    per-entry failures; other point layouts remain Unsupported, canonical absence
    Unavailable, wrong object types Corrupt. No reference-point summaries are claimed.
  - Structures: see [Structure result families](#structure-result-families).

Results-side River/Reach/Station identity is verified for every column (trimmed
comparison, original geometry strings retained); reordered identities are Corrupt.
Missing results-side tables are Unsupported for unsteady results (Python's unsteady readers
require the results-side `Cross Section Attributes`). For steady results with neither
`Results/Steady/Output/Geometry Info/Cross Section Attributes` nor the packed
`Cross Section Only`, the identity comes from `Geometry/Cross Sections/Attributes`, as in
Python `HdfResultsPlan._get_steady_cross_section_attributes`: lower-cased field lookup of
`river`, `reach` and `station` (else `rs`), with order/count alignment to the result
columns. The selected fields must be proven fixed text and agree with the River/Reach/RS
geometry identity; a field name ambiguous by case is Unsupported(IdentityLayout) rather
than last-wins. A present results-side table is validated unchanged (it requires
`Station`, where Python would also accept `RS`). The packed `Cross Section Only` fallback
accepts only the explicit 16/16/8+separator layout; unknown layouts fail closed.

This surface does not infer CRS, hydraulic units, timestamps or result success.
Zero-row/zero-section results are explicitly unsupported. Other output block names, legacy
names-only layouts and profile-variable orientation variants require new qualification.

## Time axes and reported units

`hdf::time` and `hdf::units` add read-only methods on `ReadOnlyHdfSession`; numeric result
return types are unchanged.

- `time_axis(group, expected_rows, max_bytes)` and `unsteady_time_axis(max_bytes)` read
  `Time` (`f32`/`f64` days, widened exactly), `Time Date Stamp` (proven fixed text, HEC
  `DDMMMYYYY HH:MM[:SS]`, month names case-insensitive, `24:00:00` rolls to 00:00:00 of the
  next day and is listed in `hour_2400_rows`) and `Time Date Stamp (ms)` from the base
  unsteady time series group. Real HEC-RAS files store the ms dataset as fixed strings
  `DDMMMYYYY HH:MM:SS:mmm` (e.g. `01JAN1999 12:00:00:000`; field widths 2,3,4/2,2,2,3 are
  validated, `24:00:00:000` rolls over); an integer-typed ms dataset is retained in `raw_ms`
  (monotonicity checked) but its epoch is unproven, so `integer_ms` reports Unsupported and
  no civil time is derived from it. `TimeAxis` retains the raw strings and raw integers and
  adds `ms_since_epoch` plus typed `CivilTimestamp` values. Timestamps are model-local civil
  values: no time zone, DST or leap second is invented, and `TimeEpoch::CivilUnix1970`
  declares the zone-less origin. The ms dataset is preferred when present
  (`StoredStringDataset`; `raw_ms` / `raw_ms_strings` keep the exact stored values) and,
  because `Time Date Stamp` has whole-second resolution, the two axes are compared at second
  resolution (floor), so sub-second ms parts are kept; with only strings the integers are
  `DerivedFromStrings`. `Time` days are cross-checked origin-independently: elapsed days
  from row 0 must match elapsed civil milliseconds within 1.5 ms plus 4 `f32` epsilons of
  the largest day value (1e-12 relative for a true `f64` axis), else Corrupt.
  `parse_hec_stamp` is lenient only for inner whitespace and month-name case.
- Errors: no axis dataset is NotFound; an unrecognized stamp shape, non-ASCII/vlen/compound
  text, empty axes or non-int/float dtypes are Unsupported; impossible calendar values
  (including 29 Feb in a non-leap year, hour above 24), length mismatch between axis datasets
  or against result rows, ms/string disagreement, duplicate or decreasing instants, and
  non-finite days are Corrupt. Oversized datasets are InvalidRequest. `unsteady_time_axis`
  additionally requires the axis length to equal the row count of every present 1D XS Water
  Surface/Flow and 2D Water Surface dataset (those must agree with each other).
  `unsteady_row_times(rows, max_bytes)` and `TimeAxis::select` return timestamps for stored
  row indices.
- `dataset_units_attribute(path)`, `unit_system_evidence(group)` and
  `result_units(max_bytes)` return `UnitsEvidence` (`Reported<String>`) for the `Units`
  attribute of 2D Water Surface/Depth/Velocity/Face Velocity and 1D unsteady/steady XS Water
  Surface/Flow, and for `Units System` on `/` and `/Geometry`, reported separately so
  conflicts stay visible. `UnitSystemKind` recognizes only the exact texts `US Customary` and
  `SI Metric`; anything else is `Unrecognized`. Absence is never defaulted, and no value is
  converted or inferred.
- Real HEC-RAS result datasets carry no attribute named `Units`. `dataset_units(path)` and
  `result_units(max_bytes)` therefore report, per dataset and in priority order: `Units`;
  `Variable Units` (with `Variable Name` as `quantity_name`); the two-element `Variable` =
  [name, unit] pair; then exactly one scalar fixed-text attribute whose name is a known
  quantity label (`WSEL`, `Velocity`, `Lateral Flow`, `Cumulative Volumetric Flow`, `Depth`,
  `Face Velocity`, `Flow`, `Stage`, ...; nine entries in `KNOWN_QUANTITY_LABELS`).
  `VariableUnits.source` (`UnitsSource`) names the attribute used and `raw_bytes` keeps the
  payload; several label attributes are Unsupported (ambiguous), a wrong-shaped pair or
  non-text label is Unsupported with raw bytes, and Unavailable means no unit-bearing
  attribute exists. Nothing is inferred or converted. 1D XS variables: Water Surface, Flow,
  Flow Lateral, Flow Volume Cumulative, Velocity Channel, Velocity Total.
- The quantity-label rule is applied only to XS result datasets, 2D result datasets and
  family-level storage-area datasets (where the same-named attribute holds the unit).
  Everywhere else, including every dataset below a structure group, only `Units`,
  `Variable Units` and the `Variable` pair are consulted, so descriptive attributes are never
  reported as units. `dataset_units(path) -> DatasetUnits` is the priority chain with
  provenance; `dataset_units_attribute(path) -> UnitsEvidence` is the exact `Units`
  attribute only; `dataset_unit_evidence` is a deprecated alias of `dataset_units`.
- Dataset attributes use vendored reader patch 04 (`Dataset::raw_attributes`), the dataset
  counterpart of the group raw-attribute path.
- Verification: synthetic h5py fixtures under `tests/fixtures/time/` (generator plus
  SHA-256 manifest). The ms epoch interpretation and real `Time Date Stamp (ms)` semantics
  are checked against real files by the opt-in
  `real_time_axes_and_units_match_h5py_oracle` test with `RAS_NATIVE_FIXTURES` and
  `RAS_TIME_ORACLE` (generated by `oracle_real_time_units.py`); a real-file disagreement
  surfaces as Corrupt rather than being repaired. The h5py oracle records every attribute
  and computes the expected unit source and text independently.

## Structure result families

`hdf::structures` reads the unsteady `SA 2D Area Conn`, `Lateral Structures` and
`Storage Areas` families (`StructureFamilyKind`), the only structure layouts observed in
stored HEC-RAS 5.0.7, 6.x and 7.0 result files (BaldEagleDamBrk p07/p18, FEMA eBFE AustinOyster
p03). Layouts are
never assumed; the h5py oracle `oracle_real_structures.py` records what h5py sees and the
opt-in test `real_structure_families_match_h5py_oracle_bit_exactly` (`RAS_NATIVE_FIXTURES`,
`RAS_STRUCT_ORACLE`) compares bit-exactly (IEEE bits and SHA-256 of little-endian selected
slices). 1D inline structures, bridges, culverts and pump stations are covered in
[one-d.md](one-d.md).

- `structure_inventory(kind, max_bytes)` returns `StructureInventory`: `time_rows` (from the
  validated `unsteady_time_axis`), one `StructureDescriptor` per group (exact group name,
  `Node Pointer` as `NodePointer`, `GeometryIdentity`, `series`, rank-1 `static_datasets`)
  and `family_series` (datasets directly in the family group; storage areas only). The
  result files carry no per-structure attribute table, so identity comes from the geometry
  tables: lateral groups are `River Reach RS` joined with single spaces to
  `Geometry/Structures/Attributes` rows of `Type == Lateral` (exact, unique); SA 2D
  connection groups must match exactly one `Type == Connection` row by `Connection`,
  `US SA/2D Connection` or `DS SA/2D Connection`, a pattern verified on 45 real connection
  groups across the three files but unproven in general; storage-area groups match
  `Geometry/Storage Areas/Attributes` `Name`.
- Identity rules are inferred from few real files, so a lateral group that matches no
  `Type == Lateral` row, a space-joined `River Reach RS` key collision between geometry rows,
  an unmatched or ambiguous connection group, and a storage-area group with no matching
  `Name` are Unsupported(IdentityLayout), not Corrupt. A row-count mismatch between the
  table and the groups, and duplicate trimmed group names, are Corrupt.
- A series is any rank-2 dataset below the structure group excluding the top-level
  `Geometric Info` group; its first dimension must equal the time-axis row count (Corrupt
  otherwise). Each `StructureSeries` reports path, shape, `NumericKind`, capability,
  `ColumnLabels` and `DatasetUnits`. Column labels come from `Variable_Unit` ((columns, 2)
  fixed text, name and unit) or `Connections` (names only); a wrong shape is series-level
  `ColumnLabels::Corrupt` with raw bytes, not a hidden pass. Dataset units inside a
  structure use only the exact `Units` attribute: real structure datasets carry descriptive
  attributes named after the dataset (for example `Flow = 'Flow Across the given
  Segment...'`), so quantity-label attributes are deliberately not treated as units there.
  Stored string attributes are space padded (H5T_STR_SPACEPAD) in real files; h5py
  normalizes padding on read, so raw bytes are compared per element after trimming trailing
  NUL/space.
- Storage-area family datasets (`Flow`, `Water Surface`, shape (rows, storage areas)) must
  have one column per storage-area group, but the column-to-area ordering is not proven and
  no mapping is exposed.
- `read_structure_series(kind, structure, relative, rows, columns, max_bytes)` returns exact
  `f32`/`f64` values (no conversion or transposition); `structure` is the exact group name
  (`None` only for storage-area family datasets); `relative` must not be empty, contain
  `.`/`..`/empty components or start with `Geometric Info`. `max_bytes` bounds returned
  bytes; invalid selections and group paths are InvalidRequest, absent structure/series
  NotFound, non-float dtype Unsupported, row-count disagreement with the `Time` dataset
  Corrupt.
- `plan_capabilities().structures` lists a `StructureCapability` per qualified family:
  Supported only after the full inventory validates; absent is Unavailable; failures are
  isolated per family (Corrupt for malformed storage, Unsupported(IdentityLayout) for a
  missing/unproven identity table; a geometry table over the byte budget is
  Unsupported(ResourceLimit)). I/O, internal and file-not-found failures propagate as
  errors. The aggregate `ResultFamily::Structures` entry is Corrupt if any family is
  Corrupt, else Unsupported if any is Unsupported or an unqualified group (`Structures`,
  `Inline Structures`, `Bridges`, `Culverts`, `Bridges/Culverts`, or any family under the
  steady root) is present, else Supported if any is Supported, else Unavailable.
- `StructureCapability.diagnostics` lists series whose `ColumnLabels` is Corrupt or
  Unsupported. Supported certifies identity, time-row and dtype layout only; it does not
  certify that every column-label table is valid, so inspect `diagnostics` or the
  inventory.
- Limits: 50000 structures per family, 512 series per structure, nesting depth 4
  (Unsupported ResourceLimit). Synthetic malformed cases (missing attribute table, count
  mismatch, wrong object type, duplicate names, duplicate geometry identity, duplicate node
  pointer, row count vs time axis, missing time axis, label-count mismatch, unmatched
  connection, unqualified group, storage group as dataset) live in
  `tests/fixtures/structures/` with a generator and SHA-256 manifest.

## Read families and output blocks

See [read-families.md](read-families.md) for `output_block_inventory`, `read_output_block`,
`read_family_inventory`, `read_family_dataset`, `classification_catalogue`,
`reference_point_geometry` and the terrain-modification tables: their qualification,
budgets and explicit remainders.

Pipe and 2D mesh domains require a stored `Location` plus matching geometry cardinality.
Pipe `stored_location_ids` retains integer node/conduit IDs separately from
`Identity.row` positions; no public selector silently treats IDs as positions. Generic
numeric data remains exact when a typed domain is refused.

`ReadFamily::TerrainModifications` (`terrain_modifications`, API 10) is an entry of
`READ_FAMILIES`, served by `read_family_inventory` and `read_family_dataset`. It qualifies
RASMapper `Modifications/<group>/Attributes` and
`Modifications/<group>/Control Points/Attributes` compound tables of terrain HDFs as exact
stored compounds (see
[read-families.md](read-families.md#terrain-modifications-terrain_modifications-api-10)).
`ReadFamily` is `#[non_exhaustive]`; consumers that iterate `READ_FAMILIES` pick the family
up without code changes.

`ReadFamily::MeshCornerElevations` (`mesh_corner_elevations`, API 10) lists 2D mesh corner
ground elevations: `Geometry/2D Flow Areas/<mesh>/FacePoints Elevation` and the RASMapper
mapping-cache `.../Processed Data/Profile (Horizontal)/FacePoint Elevation` in
`PostProcessing.hdf`. `ReadOnlyHdfSession::mesh_corner_elevations(mesh,
CornerElevationLayout, expected_corners, max_bytes) -> Result<Reported<CornerElevations>>`
reads one mesh in one layout with its provenance (module `hdf::corner_elevations`; see
[read-families.md](read-families.md#mesh-corner-elevations-mesh_corner_elevations-api-10)).

## Typed attributes, fixed strings and dataspaces

- `typed_dataset_attributes(path, max_bytes) -> Result<Reported<Vec<TypedAttribute>>>`:
  every attribute of one dataset, typed with the same encoding proof as family descriptors
  (exact `raw_bytes`; per-attribute unproven/damaged payloads stay in each `value`). An
  absent dataset is Unavailable; `max_bytes` bounds the summed raw attribute payloads
  (exceeded: Unsupported(resource_limit), nothing decoded); an empty, `.`/`..` or group path
  is `InvalidRequest` (groups use `typed_group_attributes`). Intended for datasets outside
  every read family, such as `SA 2D Area Conn` series and BC hydrographs. The budget is
  checked incrementally and refused before any attribute is copied or typed. It cannot
  bound the object-header parse: the reader materializes every attribute message (dense
  storage included) when the dataset is opened, as for every dataset read.
- `raw_fixed_strings(path, max_bytes) -> Result<Reported<RawFixedStrings>>`: a fixed-length
  string dataset exactly as stored (`shape`, `width`, `padding`, `charset`, row-major
  `raw_bytes`, padding retained). Non-text or variable-length storage is
  Unsupported(encoding); the stored-byte budget is checked before reading; a payload that
  disagrees with shape x width is Corrupt. h5py reads into NULLPAD memory, so its bytes are
  the stored bytes with SPACEPAD trailing spaces / NULLTERM post-NUL bytes replaced by NULs;
  NULLPAD is unchanged. Real 5.0.7 SPACEPAD sources can also hold unwritten NUL fill.
- `AttributeType::Bitfield { size, order }` and `MetadataValue::Bitfield(Vec<u8>)` type
  proven HDF5 bitfield attributes and compound members (see
  [read-families.md](read-families.md)).
- **Text decoding matches h5py.** Typed metadata strings (`TypedAttribute`, compound
  members, `typed_*_attributes`) and `read_strings` drop SPACEPAD trailing spaces and *then*
  trailing NULs (unwritten fill), as HDF5's NULLPAD conversion plus NumPy `S` trimming do:
  NULLTERM keeps bytes before the first NUL, NULLPAD drops trailing NULs. `raw_bytes` and
  `raw_fixed_strings` stay exact stored bytes. Before API 9 a SPACEPAD element of NUL fill
  decoded to NUL characters where h5py returns `''` (11 real attributes: Baxter/Orleans
  `Variable_Unit`, Weise `Transport Function`). Corpus check: 25,780/25,780 dataset and
  7,353/7,353 group attributes match h5py; of 21,160 datasets read through `read_strings`,
  `simple_compound` and the numeric readers, only 20 string results changed, all to h5py's.
  `read_strings` refuses non-UTF-8 bytes in
  fixed-length text (Corrupt; e.g. a 5.0.7 `Geometry Info/Node Info` table), where h5py returns the
  bytes. Variable-length string attributes, including a vlen `Location`, stay
  Unsupported(encoding); see the divergence table in [mesh-results.md](mesh-results.md).
- **Links.** Soft links to datasets are followed in new-style and old-style (symbol-table)
  groups, including the root group. A dangling soft link reads as absent; external links are
  not followed and also read as absent (Unavailable) in the path-addressed readers.

`hdf::Dataspace { Scalar, Simple(dims), Null }` (`as_str`: `scalar`/`simple`/`null`) is
explicit on `TypedAttribute`, `NumericCapability`, `StoredDataset`, `CompoundTable` and
`RawFixedStrings`. An H5S_NULL dataspace (h5py `Empty(dtype)`) is not confused with a
zero-length simple array (`array([], dtype)`) or a scalar:

- Attributes keep their legacy `shape` (`[]` scalar, `[0]` NULL, dims otherwise) and empty
  NULL values; `dataspace` is the authority for NULL versus empty.
- Dataset `shape` is the stored dims (`[]` for both scalar and NULL); consult `dataspace`.
- `raw_fixed_strings` and `simple_compound` report a NULL dataset as Supported with
  `Dataspace::Null` and no elements.
- `numeric_capability` reports a NULL numeric dataset Unsupported(schema), and
  `read_numeric`/`read_strings` refuse NULL with Unsupported(schema); numeric arrays and
  string lists cannot represent `Empty`.

The opt-in `typed_attributes` oracle test (`RAS_RAW_PAYLOAD_ORACLE`, generated by
`tests/fixtures/raw_payload_oracle.py`) compares every bitfield attribute and every
fixed-string dataset (up to 1 MiB) of the real corpus with h5py byte for byte.

## Session caching

See [perf-session.md](perf-session.md) for exact-key memoization, owned outputs, shared
worker-count pools, cache lifetime and measurement caveats. Explicit thread-count semantics
(1..1024 workers) are unchanged by caching.

## Known gaps

Typed generic shared-message error codes, semantic CRS validation and authority/unit
inference are not implemented. Other gaps are listed per family on the linked pages.

## API version history

`API_VERSION` advances for breaking changes and for additive capabilities.

| API | Added or changed |
|---|---|
| 5 | Unified evidence and errors. `Reported<T>` replaces the ad hoc evidence enums (`XsField`, `UnitsEvidence`, `ColumnLabels`, `NodePointer` become aliases; use `value()`, `raw_bytes()`, `capability()`, `is_unavailable()`); `MetadataValue`, `AttributeType` and `EvidenceStatus` Unsupported/Corrupt payloads change shape. `mesh_inventory`, `mesh_name_diagnostics`, `plan_capabilities`, `steady_profile_names` and `result_units` take `max_bytes`; `read_cross_section_result` takes `metadata_max_bytes` before `max_bytes`. `CrossSection.index`, `GeometryIdentity.row` and `MeshDescriptor.source` are replaced by `id: Identity`. `dataset_units` returns `DatasetUnits`, the exact attribute is `dataset_units_attribute`, `dataset_unit_evidence` is deprecated; `MeshResultMode` and `XsResultKind` become aliases of `hdf::ResultMode`. Unsupported codes are no longer defaulted; `Internal`/`FileNotFound` propagate from every family check; reasons have no `Kind:` prefix; `CivilTimestamp::new` and `relative_projection` escapes are `InvalidRequest`; a duplicated `Units System` is Corrupt evidence; unsteady XS reads check the stored `Time` row count; only a ResourceLimit-coded `InvalidRequest` becomes a status. Additions: `Error.code`, `Error::unsupported`, `Reported`, `ColumnLabelSet`, `Identity`, `ResultMode`, `as_str()` spellings, `DEFAULT_MAX_BYTES`, `API_VERSION`, `capabilities()`, `api::ENTRYPOINTS`. Also within API 5: exact mesh geometry, geometry families, mesh identity comparison, the layer catalogue and the stored mesh result reads ([mesh-results.md](mesh-results.md)). |
| 6 | Behaviour: requested-only BC reads (including named subsets) return valid data when an unrelated sibling line is corrupt (inventory and all-lines batches still report it); a zero-byte chunk cache also disables the optional hot full-array copy; non-inline extensible-array chunk indexes are refused (Unsupported) instead of risking fill values. Additions: `with_cache_options`, `chunk_cache_stats`, `read_mesh_summary_values`, `read_mesh_boundary_batch`. |
| 7 | Additive: 1D cross-section geometry, line families and topology, XS result variables, 1D Summary Output, 1D structure series and structure/pump/storage-area geometry groups, and their layer-catalogue entries ([one-d.md](one-d.md)). `layer_inventory` returns additional layers. |
| 8 | Additive: `output_block_inventory`, `read_output_block`, `read_family_inventory`, `read_family_dataset`, `classification_catalogue` and `reference_point_geometry` ([read-families.md](read-families.md)): stored family and byte-bitfield envelopes, environmental classifications, explicit per-object capabilities and source identities. Plan/run metadata remain raw attribute/text/table envelopes. Pipe and mesh domains require stored `Location` plus matching geometry cardinality, so missing `Location` may refuse previously inferred typed results. |
| 9 | Additive: `typed_dataset_attributes`, `raw_fixed_strings`, `AttributeType::Bitfield`/`MetadataValue::Bitfield`, `hdf::Dataspace`, `mesh_results::DomainRule` and `MeshResultVariable.domain_rule`. Behaviour: the absent-Location legacy exact-name rule for six Base Output 2D series names, with the physical `Cell Count` bound applied to both domain paths (mirroring ras-commander PR #565); mesh series require the Base Output `Time` dataset; text decoding matches h5py; soft links are followed in old-style groups; NULL dataspaces are distinguished. |
| 10 | Behaviour: mesh ring order is a contract (clockwise, first listed face, duplicates dropped, fewer than three faces omitted). Additive within API 10: whole-read budgets (`DEFAULT_READ_MAX_BYTES`, `read_numeric_bounded`, `read_numeric_batch_bounded`, `read_topology_bounded`), the `terrain_modifications` read family, and `Topology::face_lines` / `read_mesh_perimeter`. Behaviour within API 10: HEC-RAS 5.0 row-label water-surface summaries qualify as cell summaries (`DomainRule::LegacyRowLabels`, see [mesh-results.md](mesh-results.md)). Additive within API 10: `ReadOnlyHdfSession::from_bytes` (in-memory sessions, see [sessions.md](sessions.md)). Additive within API 10: the `mesh_corner_elevations` read family and `ReadOnlyHdfSession::mesh_corner_elevations`. Additive within API 10: `read_mesh_face_lines` (face lines from the four face tables only). |
