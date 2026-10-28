//! HEC-RAS layers as Arrow record batches in the canonical layer schema (feature `arrow`).
//!
//! The engine owns the layer catalog: layer names, geometry types, identity fields and field
//! types. Consumers alias it (ras2cng archives, terrio's HECRAS driver); see [`CATALOG`].
//! Geometry is little-endian 2D OGC WKB in a `geometry` Binary field tagged
//! `ARROW:extension:name = geoarrow.wkb`. The engine attaches no CRS: CRS evidence is the
//! caller's (the HDF `Projection` text is available through [`crate::hdf`]).
//!
//! Every layer reproduces what ras-commander's Python readers return for the same file
//! (rows, row order, column order, dtypes, values and WKB), or refuses with a typed error.
//! The conditions under which ras-commander itself would leave its native path (and so might
//! return different geometry) are refusals here too.
use crate::hdf::metadata::MetadataValue;
use crate::hdf::{NumericValues, ReadOnlyHdfSession, DEFAULT_MAX_BYTES, DEFAULT_READ_MAX_BYTES};
use crate::mesh::{GeometryPolicy, OmissionKind};
use crate::{Error, ErrorKind, Result, UnsupportedCode};
use arrow_array::builder::{BinaryBuilder, StringBuilder};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Float32Array, Float64Array, Int64Array, RecordBatch,
    TimestampNanosecondArray, UInt64Array,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const GEOMETRY: &str = "Geometry/2D Flow Areas";
/// Extension name of the WKB geometry field.
pub const WKB_EXTENSION: &str = "geoarrow.wkb";

/// Implementation status of a catalog layer in this engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayerStatus {
    /// Returned as Arrow by this module.
    Arrow,
    /// Named by the catalog; read through the engine's typed family APIs, not yet as Arrow.
    Planned,
}

/// One layer of the canonical catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LayerDef {
    /// Canonical name.
    pub name: &'static str,
    /// OGC geometry type of every feature.
    pub geometry: &'static str,
    /// Row identity: namespace field and optional local integer field.
    pub identity: (&'static str, Option<&'static str>),
    /// Names consumers use for the same layer: `(consumer, name)`.
    pub aliases: &'static [(&'static str, &'static str)],
    pub status: LayerStatus,
}

const fn def(
    name: &'static str,
    geometry: &'static str,
    identity: (&'static str, Option<&'static str>),
    aliases: &'static [(&'static str, &'static str)],
    status: LayerStatus,
) -> LayerDef {
    LayerDef {
        name,
        geometry,
        identity,
        aliases,
        status,
    }
}

use LayerStatus::{Arrow as A, Planned as P};
const ROW: (&str, Option<&str>) = ("source_row", None);
/// The canonical layer catalog. Mesh summary results are one further layer per stored
/// variable ([`ReadOnlyHdfSession::mesh_summary_layer`]), named by [`variable_slug`].
pub const CATALOG: &[LayerDef] = &[
    def(
        "mesh_cells",
        "Polygon",
        ("mesh_name", Some("cell_id")),
        &[],
        A,
    ),
    def(
        "mesh_cell_points",
        "Point",
        ("mesh_name", Some("cell_id")),
        &[],
        P,
    ),
    def(
        "mesh_faces",
        "LineString",
        ("mesh_name", Some("face_id")),
        &[],
        A,
    ),
    def(
        "mesh_perimeters",
        "Polygon",
        ("mesh_name", None),
        &[("ras2cng", "mesh_areas")],
        A,
    ),
    def("cross_sections", "LineString", ROW, &[], P),
    def(
        "river_centerlines",
        "LineString",
        ROW,
        &[("ras2cng", "centerlines")],
        P,
    ),
    def(
        "river_bank_lines",
        "LineString",
        ROW,
        &[("ras2cng", "bank_lines")],
        P,
    ),
    def(
        "river_edge_lines",
        "LineString",
        ROW,
        &[("ras2cng", "edge_lines")],
        P,
    ),
    def("river_flow_paths", "LineString", ROW, &[], P),
    def("river_junctions", "Point", ROW, &[], P),
    def("storage_areas", "Polygon", ROW, &[], P),
    def("structures", "LineString", ROW, &[], P),
    def("bc_lines", "LineString", ROW, &[], P),
    def("breaklines", "LineString", ROW, &[], P),
    def("reference_lines", "LineString", ROW, &[], P),
    def("reference_points", "Point", ROW, &[], P),
    def("refinement_regions", "Polygon", ROW, &[], P),
    def("pump_stations", "Point", ROW, &[], P),
    def("mannings_n_regions", "Polygon", ROW, &[], P),
];

/// The catalog entry for a canonical name or any consumer alias.
pub fn lookup(name: &str) -> Option<&'static LayerDef> {
    CATALOG
        .iter()
        .find(|d| d.name == name || d.aliases.iter().any(|(_, a)| *a == name))
}

/// Field name of a summary variable: lower case, spaces to underscores (ras-commander's
/// `var.lower().replace(" ", "_")`). Only ASCII names are accepted; anything else is
/// refused rather than lower-cased by a second Unicode rule.
pub fn variable_slug(variable: &str) -> Result<String> {
    if variable.is_empty() || !variable.is_ascii() {
        return Err(Error::unsupported(
            UnsupportedCode::Encoding,
            format!("summary variable name {variable:?} is not plain ASCII"),
        ));
    }
    Ok(variable.to_ascii_lowercase().replace(' ', "_"))
}

/// Read options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LayerOptions {
    /// Maximum rows per record batch.
    pub batch_rows: usize,
    /// Budget for one mesh's arrays.
    pub max_bytes: usize,
    /// Budget for metadata tables and attributes.
    pub metadata_max_bytes: usize,
}
impl Default for LayerOptions {
    fn default() -> Self {
        Self {
            batch_rows: 65_536,
            max_bytes: DEFAULT_READ_MAX_BYTES,
            metadata_max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

impl LayerOptions {
    /// Options with `batch_rows` rows per record batch.
    pub fn with_batch_rows(mut self, batch_rows: usize) -> Self {
        self.batch_rows = batch_rows;
        self
    }
    /// Options with these byte budgets (per mesh array read, and per metadata object).
    pub fn with_budgets(mut self, max_bytes: usize, metadata_max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self.metadata_max_bytes = metadata_max_bytes;
        self
    }
}

/// One layer: its schema and batches (each at most `batch_rows` rows).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ArrowLayer {
    pub name: String,
    pub schema: SchemaRef,
    pub batches: Vec<RecordBatch>,
    /// Identity fields of every row: namespace and optional local integer.
    pub identity: (String, Option<String>),
}
impl ArrowLayer {
    pub fn num_rows(&self) -> usize {
        self.batches.iter().map(RecordBatch::num_rows).sum()
    }
    fn from_columns(
        name: &str,
        fields: Vec<Field>,
        columns: Vec<ArrayRef>,
        identity: (&str, Option<&str>),
        batch_rows: usize,
    ) -> Result<Self> {
        if batch_rows == 0 {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "batch_rows must be positive",
            ));
        }
        let schema = Arc::new(Schema::new(fields));
        let all = RecordBatch::try_new(schema.clone(), columns).map_err(arrow_error)?;
        let mut batches = Vec::new();
        let mut start = 0;
        while start < all.num_rows() {
            let len = batch_rows.min(all.num_rows() - start);
            batches.push(all.slice(start, len));
            start += len;
        }
        Ok(Self {
            name: name.to_owned(),
            schema,
            batches,
            identity: (identity.0.to_owned(), identity.1.map(str::to_owned)),
        })
    }
}

fn arrow_error(e: arrow_schema::ArrowError) -> Error {
    Error::new(ErrorKind::Internal, format!("arrow: {e}"))
}
fn refuse(code: UnsupportedCode, m: impl Into<String>) -> Error {
    Error::unsupported(code, m)
}

/// The `geometry` field: nullable WKB with the GeoArrow extension name.
pub fn geometry_field() -> Field {
    Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
        "ARROW:extension:name".to_owned(),
        WKB_EXTENSION.to_owned(),
    )]))
}

// --- WKB -------------------------------------------------------------------------------

fn put_point(out: &mut Vec<u8>, p: [f64; 2]) {
    out.extend_from_slice(&p[0].to_le_bytes());
    out.extend_from_slice(&p[1].to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, n: usize) -> Result<()> {
    let n = u32::try_from(n)
        .map_err(|_| refuse(UnsupportedCode::ResourceLimit, "WKB count exceeds u32"))?;
    out.extend_from_slice(&n.to_le_bytes());
    Ok(())
}
/// Little-endian 2D WKB polygon with one ring (as Shapely writes it).
pub fn wkb_polygon(out: &mut Vec<u8>, ring: &[[f64; 2]]) -> Result<()> {
    out.push(1);
    out.extend_from_slice(&3u32.to_le_bytes());
    put_u32(out, 1)?;
    put_u32(out, ring.len())?;
    ring.iter().for_each(|p| put_point(out, *p));
    Ok(())
}
/// Little-endian 2D WKB line string.
pub fn wkb_line(out: &mut Vec<u8>, line: &[[f64; 2]]) -> Result<()> {
    out.push(1);
    out.extend_from_slice(&2u32.to_le_bytes());
    put_u32(out, line.len())?;
    line.iter().for_each(|p| put_point(out, *p));
    Ok(())
}

// --- Mesh names --------------------------------------------------------------------------

/// Whether ras-commander's `HdfUtils.convert_ras_string` returns `name` unchanged as text.
/// It turns `True`/`False`, `ddMMMyyyy HH:MM:SS...`, `ddMMMyyyy HHMM...` and `HH:MM:SS` into
/// other types; such names are refused.
fn plain_ras_text(s: &str) -> bool {
    if s == "True" || s == "False" {
        return false;
    }
    let b = s.as_bytes();
    let d = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    // Python's \w is Unicode-aware; any non-space, non-punctuation char could match. Treat
    // every non-ASCII-punctuation, non-space byte as a possible \w (conservative refusal).
    let w = |i: usize| {
        b.get(i)
            .is_some_and(|c| !c.is_ascii_whitespace() && !c.is_ascii_punctuation())
    };
    let date = d(0)
        && d(1)
        && w(2)
        && w(3)
        && w(4)
        && d(5)
        && d(6)
        && d(7)
        && d(8)
        && b.get(9) == Some(&b' ');
    if date && d(10) && d(11) {
        return false;
    }
    let duration = b.len() == 8
        && d(0)
        && d(1)
        && b[2] == b':'
        && d(3)
        && d(4)
        && b[5] == b':'
        && d(6)
        && d(7);
    !duration
}

impl ReadOnlyHdfSession {
    /// 2D flow area names in `Geometry/2D Flow Areas/Attributes` order, exactly as
    /// ras-commander lists them (`HdfMesh.get_mesh_area_names`). An absent
    /// `Geometry/2D Flow Areas` group gives no names. A missing Attributes table (where
    /// ras-commander logs an error and returns nothing), a first column other than `Name`,
    /// empty or duplicate names and names ras-commander would convert to non-text are refused.
    pub fn mesh_area_names(&self, metadata_max_bytes: usize) -> Result<Vec<String>> {
        match self.group_members(GEOMETRY) {
            Ok(_) => {}
            Err(e) if e.kind == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        }
        let table = self.simple_compound(&format!("{GEOMETRY}/Attributes"), metadata_max_bytes)?;
        let first = table.columns.first().ok_or_else(|| {
            refuse(
                UnsupportedCode::Schema,
                "2D Flow Areas Attributes has no columns",
            )
        })?;
        if first.name != "Name" {
            return Err(refuse(
                UnsupportedCode::Schema,
                "2D Flow Areas Attributes: first column is not Name",
            ));
        }
        let MetadataValue::Strings(names) = &first.value else {
            return Err(refuse(
                UnsupportedCode::Encoding,
                "mesh Name requires proven fixed strings",
            ));
        };
        let mut seen = HashSet::new();
        for n in names {
            if n.is_empty()
                || n.contains(['/', '\\'])
                || !seen.insert(n.as_str())
                || !plain_ras_text(n)
            {
                return Err(refuse(
                    UnsupportedCode::Schema,
                    format!("mesh name {n:?} is empty, duplicated, not one group component or not plain text"),
                ));
            }
        }
        Ok(names.clone())
    }

    fn mesh_datasets(&self, mesh: &str) -> Result<HashSet<String>> {
        Ok(self
            .group_members(&format!("{GEOMETRY}/{mesh}"))?
            .datasets
            .into_iter()
            .map(|d| d.rsplit('/').next().unwrap_or(&d).to_owned())
            .collect())
    }

    /// `mesh_cells`: one Polygon per assembled cell (`mesh_name`, `cell_id`, `geometry`), meshes
    /// in Attributes order, cells in stored order; rings as [`crate::mesh::Topology::assemble`]
    /// builds them, closed. This is ras-commander's native `HdfMesh.get_mesh_cell_polygons`.
    /// Refused where ras-commander would use its Python polygonizer instead: a mesh without the
    /// full topology arrays, a cell quarantined as invalid geometry, or an assembled cell with
    /// fewer than three faces.
    pub fn mesh_cells_layer(&self, options: &LayerOptions) -> Result<ArrowLayer> {
        let mut names = StringBuilder::new();
        let mut ids = Vec::new();
        let mut geoms = BinaryBuilder::new();
        let mut wkb = Vec::new();
        for mesh in self.mesh_area_names(options.metadata_max_bytes)? {
            let present = self.mesh_datasets(&mesh)?;
            let needed = [
                "FacePoints Coordinate",
                "Faces FacePoint Indexes",
                "Faces Perimeter Info",
                "Faces Perimeter Values",
                "Cells Face and Orientation Info",
                "Cells Face and Orientation Values",
            ];
            if !needed.iter().all(|n| present.contains(*n)) {
                return Err(refuse(
                    UnsupportedCode::Schema,
                    format!("{mesh}: incomplete mesh topology (ras-commander uses its Python polygonizer)"),
                ));
            }
            let named = self.read_topology_bounded(&mesh, options.max_bytes)?;
            let packed = named
                .topology
                .assemble_with_session(GeometryPolicy::QuarantineInvalid, self)?;
            if let Some(d) = packed
                .diagnostics
                .iter()
                .find(|d| d.kind == OmissionKind::InvalidGeometry)
            {
                return Err(refuse(
                    UnsupportedCode::NotImplemented,
                    format!("{mesh}: cell {} omitted as invalid geometry ({}); ras-commander's Python polygonizer is not reproduced", d.cell_id, d.reason),
                ));
            }
            let info = &named.topology.cell_info;
            for (row, &cell) in packed.cell_ids.iter().enumerate() {
                let c = usize::try_from(cell)
                    .map_err(|_| Error::new(ErrorKind::Internal, "cell id"))?;
                if info[[c, 1]] < 3 {
                    return Err(refuse(
                        UnsupportedCode::NotImplemented,
                        format!("{mesh}: cell {cell} assembled from fewer than three faces"),
                    ));
                }
                let (a, b) = (
                    packed.offsets[row] as usize,
                    packed.offsets[row + 1] as usize,
                );
                let ring = &packed.coordinates[a..b];
                wkb.clear();
                if ring.first() != ring.last() {
                    let mut closed = ring.to_vec();
                    closed.push(ring[0]);
                    wkb_polygon(&mut wkb, &closed)?;
                } else {
                    wkb_polygon(&mut wkb, ring)?;
                }
                geoms.append_value(&wkb);
                names.append_value(&mesh);
                ids.push(cell);
            }
        }
        ArrowLayer::from_columns(
            "mesh_cells",
            vec![
                Field::new("mesh_name", DataType::Utf8, false),
                Field::new("cell_id", DataType::Int64, false),
                geometry_field(),
            ],
            vec![
                Arc::new(names.finish()),
                Arc::new(Int64Array::from(ids)),
                Arc::new(geoms.finish()),
            ],
            ("mesh_name", Some("cell_id")),
            options.batch_rows,
        )
    }

    /// `mesh_faces`: every face as a LineString (`mesh_name`, `face_id`, `geometry`), meshes in
    /// Attributes order, faces in stored order ([`Self::read_mesh_face_lines`]). A mesh with
    /// none of the four face tables is skipped, as ras-commander does; a partial set is refused.
    pub fn mesh_faces_layer(&self, options: &LayerOptions) -> Result<ArrowLayer> {
        let mut names = StringBuilder::new();
        let mut ids = Vec::new();
        let mut geoms = BinaryBuilder::new();
        let mut wkb = Vec::new();
        for mesh in self.mesh_area_names(options.metadata_max_bytes)? {
            let present = self.mesh_datasets(&mesh)?;
            let needed = [
                "Faces FacePoint Indexes",
                "FacePoints Coordinate",
                "Faces Perimeter Info",
                "Faces Perimeter Values",
            ];
            let count = needed.iter().filter(|n| present.contains(**n)).count();
            if count == 0 {
                continue;
            }
            if count != needed.len() {
                return Err(refuse(
                    UnsupportedCode::Schema,
                    format!("{mesh}: face topology is partial"),
                ));
            }
            let lines = self.read_mesh_face_lines(&mesh, options.max_bytes)?;
            for (row, &face) in lines.face_ids.iter().enumerate() {
                let (a, b) = (lines.offsets[row] as usize, lines.offsets[row + 1] as usize);
                wkb.clear();
                wkb_line(&mut wkb, &lines.coordinates[a..b])?;
                geoms.append_value(&wkb);
                names.append_value(&mesh);
                ids.push(face);
            }
        }
        ArrowLayer::from_columns(
            "mesh_faces",
            vec![
                Field::new("mesh_name", DataType::Utf8, false),
                Field::new("face_id", DataType::Int64, false),
                geometry_field(),
            ],
            vec![
                Arc::new(names.finish()),
                Arc::new(Int64Array::from(ids)),
                Arc::new(geoms.finish()),
            ],
            ("mesh_name", Some("face_id")),
            options.batch_rows,
        )
    }

    /// `mesh_perimeters` (ras2cng `mesh_areas`): each 2D flow area's stored `Perimeter` as a
    /// closed Polygon (`mesh_name`, `geometry`), Attributes order ([`Self::read_mesh_perimeter`]).
    /// An area without `Perimeter` (ras-commander's collection-level fallback) is refused.
    pub fn mesh_perimeters_layer(&self, options: &LayerOptions) -> Result<ArrowLayer> {
        let mut names = StringBuilder::new();
        let mut geoms = BinaryBuilder::new();
        let mut wkb = Vec::new();
        for mesh in self.mesh_area_names(options.metadata_max_bytes)? {
            let p = self
                .read_mesh_perimeter(&mesh, options.max_bytes)
                .map_err(|e| {
                    if e.kind == ErrorKind::NotFound {
                        refuse(
                            UnsupportedCode::NotImplemented,
                            format!("{mesh}: no Perimeter dataset"),
                        )
                    } else {
                        e
                    }
                })?;
            wkb.clear();
            wkb_polygon(&mut wkb, &p.ring)?;
            geoms.append_value(&wkb);
            names.append_value(&mesh);
        }
        ArrowLayer::from_columns(
            "mesh_perimeters",
            vec![
                Field::new("mesh_name", DataType::Utf8, false),
                geometry_field(),
            ],
            vec![Arc::new(names.finish()), Arc::new(geoms.finish())],
            ("mesh_name", None),
            options.batch_rows,
        )
    }

    /// Simulation start of a plan: `Plan Data/Plan Information` attribute
    /// `Simulation Start Time`, strictly `ddMMMyyyy HH:MM:SS` (what Python's
    /// `strptime("%d%b%Y %H:%M:%S")` reads in ras-commander), as civil nanoseconds since
    /// 1970-01-01. Any other spelling, and the HEC-RAS 5.0 fallbacks, are refused.
    pub fn simulation_start_ns(&self) -> Result<i64> {
        let attrs = self.typed_group_attributes("Plan Data/Plan Information")?;
        let attr = attrs
            .iter()
            .find(|a| a.name == "Simulation Start Time")
            .ok_or_else(|| {
                refuse(
                    UnsupportedCode::Schema,
                    "no Simulation Start Time attribute",
                )
            })?;
        let text = match &attr.value {
            MetadataValue::Strings(v) if v.len() == 1 => v[0].clone(),
            _ => {
                return Err(refuse(
                    UnsupportedCode::Encoding,
                    "Simulation Start Time is not one string",
                ))
            }
        };
        let b = text.as_bytes();
        let shape = text.len() == 18
            && b[9] == b' '
            && b[12] == b':'
            && b[15] == b':'
            && [0, 1, 5, 6, 7, 8, 10, 11, 13, 14, 16, 17]
                .iter()
                .all(|&i| b[i].is_ascii_digit())
            && b[2..5].iter().all(u8::is_ascii_alphabetic);
        if !shape {
            return Err(refuse(
                UnsupportedCode::Encoding,
                format!("Simulation Start Time {text:?} is not ddMMMyyyy HH:MM:SS"),
            ));
        }
        let parsed = crate::hdf::time::parse_hec_stamp(&text)?;
        if parsed.hour_2400 {
            return Err(refuse(
                UnsupportedCode::Encoding,
                "24:00 start time is not read by strptime",
            ));
        }
        parsed
            .timestamp
            .ms_since_epoch()
            .checked_mul(1_000_000)
            .ok_or_else(|| {
                refuse(
                    UnsupportedCode::ResourceLimit,
                    "start time outside nanosecond range",
                )
            })
    }

    /// One mesh summary variable as the canonical results layer named by [`variable_slug`]:
    /// `mesh_name`, `cell_id` or `face_id`, `<slug>` (stored type; integers as Int64, or
    /// UInt64 when stored unsigned; NaN as null), `<slug>_time` (when the engine proves
    /// row 1 of a two-row summary is `Time`: simulation start plus the stored days, rounded
    /// half to even to 100 ms; Timestamp(us) when every stored offset is a whole number of
    /// days, as pandas 3 infers, Timestamp(ns) otherwise) and, for cells, `is_ghost_cell`,
    /// with ghost cells (index at or above `Cell Count`) left out. Meshes in Attributes order;
    /// a mesh that does not store the variable is skipped; a variable no mesh stores is
    /// `NotFound`. This is ras-commander's
    /// `HdfResultsMesh.get_mesh_summary_output(..., include_geometry=False)` with the column
    /// types its ras2cng archive stores.
    pub fn mesh_summary_layer(&self, variable: &str, options: &LayerOptions) -> Result<ArrowLayer> {
        let slug = variable_slug(variable)?;
        let face = variable.contains("Face");
        let id_field = if face { "face_id" } else { "cell_id" };
        let mut start: Option<i64> = None;
        let mut names = StringBuilder::new();
        let mut ids: Vec<i64> = Vec::new();
        let mut values: Vec<Option<f64>> = Vec::new();
        let mut ints: Vec<i128> = Vec::new();
        let mut kind: Option<&'static str> = None;
        let mut times: Vec<Option<i64>> = Vec::new();
        let mut has_time: Option<bool> = None;
        let mut micro: Option<bool> = None;
        let mut found = false;
        let mut ghost: Vec<bool> = Vec::new();
        for mesh in self.mesh_area_names(options.metadata_max_bytes)? {
            let inventory = self.mesh_summary_inventory(&mesh, options.metadata_max_bytes)?;
            if !inventory.capability.is_supported() {
                continue;
            }
            let Some(v) = inventory.variables.iter().find(|v| v.name == variable) else {
                continue;
            };
            if !v.capability.is_supported() {
                return Err(refuse(
                    UnsupportedCode::Schema,
                    format!("{mesh}/{variable}: {:?}", v.capability),
                ));
            }
            let engine_face = v.domain == crate::hdf::mesh_results::ElementDomain::Face;
            let engine_cell = v.domain == crate::hdf::mesh_results::ElementDomain::Cell;
            if (face && !engine_face) || (!face && !engine_cell) {
                return Err(refuse(
                    UnsupportedCode::Schema,
                    format!("{mesh}/{variable}: element domain {:?} differs from ras-commander's name rule", v.domain),
                ));
            }
            let array = self.read_mesh_summary_values(
                &mesh,
                variable,
                None,
                options.metadata_max_bytes,
                options.max_bytes,
            )?;
            let (rows, n) = match array.shape.as_slice() {
                [n] => (1u64, *n),
                [2, n] => (2, *n),
                s => {
                    return Err(refuse(
                        UnsupportedCode::Schema,
                        format!("{mesh}/{variable}: shape {s:?}"),
                    ))
                }
            };
            let timed = rows == 2;
            if timed && v.time_of_extreme_row != Some(1) {
                return Err(refuse(
                    UnsupportedCode::Schema,
                    format!("{mesh}/{variable}: row 1 is not a proven Time row"),
                ));
            }
            if *has_time.get_or_insert(timed) != timed {
                return Err(refuse(
                    UnsupportedCode::Schema,
                    format!("{variable}: meshes differ in rows"),
                ));
            }
            let n = usize::try_from(n)
                .map_err(|_| refuse(UnsupportedCode::ResourceLimit, "element count"))?;
            let this_kind: &'static str = match &array.values {
                NumericValues::F32(_) => "f32",
                NumericValues::F64(_) => "f64",
                NumericValues::U8(_)
                | NumericValues::U16(_)
                | NumericValues::U32(_)
                | NumericValues::U64(_) => "u",
                _ => "i",
            };
            if timed && !matches!(this_kind, "f32" | "f64") {
                return Err(refuse(
                    UnsupportedCode::Schema,
                    format!("{mesh}/{variable}: timed integers"),
                ));
            }
            if *kind.get_or_insert(this_kind) != this_kind {
                return Err(refuse(
                    UnsupportedCode::Schema,
                    format!("{variable}: meshes store different types"),
                ));
            }
            let flat: Vec<Option<f64>>;
            let mut flat_int: Vec<i128> = Vec::new();
            match array.values {
                NumericValues::F32(v) => flat = v.into_iter().map(|x| Some(f64::from(x))).collect(),
                NumericValues::F64(v) => flat = v.into_iter().map(Some).collect(),
                NumericValues::I8(v) => {
                    flat_int = v.into_iter().map(i128::from).collect();
                    flat = vec![];
                }
                NumericValues::I16(v) => {
                    flat_int = v.into_iter().map(i128::from).collect();
                    flat = vec![];
                }
                NumericValues::I32(v) => {
                    flat_int = v.into_iter().map(i128::from).collect();
                    flat = vec![];
                }
                NumericValues::I64(v) => {
                    flat_int = v.into_iter().map(i128::from).collect();
                    flat = vec![];
                }
                NumericValues::U8(v) => {
                    flat_int = v.into_iter().map(i128::from).collect();
                    flat = vec![];
                }
                NumericValues::U16(v) => {
                    flat_int = v.into_iter().map(i128::from).collect();
                    flat = vec![];
                }
                NumericValues::U32(v) => {
                    flat_int = v.into_iter().map(i128::from).collect();
                    flat = vec![];
                }
                NumericValues::U64(v) => {
                    flat_int = v.into_iter().map(i128::from).collect();
                    flat = vec![];
                }
            }
            let physical = if face {
                None
            } else {
                self.physical_cell_count(&mesh, options.metadata_max_bytes)?
            };
            if timed && start.is_none() {
                start = Some(self.simulation_start_ns()?);
            }
            if timed {
                // pandas 3 infers microsecond resolution for a mesh whose day offsets are all
                // whole numbers (or NaN), nanoseconds otherwise; concatenating meshes of
                // different resolutions is not reproduced.
                let whole = flat[n..]
                    .iter()
                    .all(|d| d.is_none_or(|x| x.is_nan() || x.fract() == 0.0));
                if *micro.get_or_insert(whole) != whole {
                    return Err(refuse(
                        UnsupportedCode::NotImplemented,
                        format!(
                            "{variable}: meshes differ in time resolution (whole days in some)"
                        ),
                    ));
                }
            }
            found = true;
            for i in 0..n {
                let is_ghost = physical.is_some_and(|c| i as u64 >= c);
                if is_ghost {
                    continue;
                }
                names.append_value(&mesh);
                ids.push(
                    i64::try_from(i).map_err(|_| refuse(UnsupportedCode::ResourceLimit, "id"))?,
                );
                if flat.is_empty() && !flat_int.is_empty() {
                    ints.push(flat_int[i]);
                } else {
                    values.push(flat[i]);
                }
                if timed {
                    let days =
                        flat[n + i].ok_or_else(|| Error::new(ErrorKind::Internal, "time"))?;
                    times.push(timestamp_ns(start.unwrap_or_default(), days)?);
                }
                if !face {
                    ghost.push(false);
                }
            }
        }
        if !found {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("no 2D flow area stores the summary variable {variable:?}"),
            ));
        }
        let mut fields = vec![
            Field::new("mesh_name", DataType::Utf8, false),
            Field::new(id_field, DataType::Int64, false),
        ];
        let mut columns: Vec<ArrayRef> =
            vec![Arc::new(names.finish()), Arc::new(Int64Array::from(ids))];
        let value: ArrayRef = match kind.unwrap_or("f32") {
            "f32" => Arc::new(Float32Array::from(
                values
                    .iter()
                    .map(|v| v.filter(|x| !x.is_nan()).map(|x| x as f32))
                    .collect::<Vec<_>>(),
            )),
            "f64" => Arc::new(Float64Array::from(
                values
                    .iter()
                    .map(|v| v.filter(|x| !x.is_nan()))
                    .collect::<Vec<_>>(),
            )),
            "u" => Arc::new(UInt64Array::from(
                ints.iter()
                    .map(|&v| u64::try_from(v))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|_| Error::new(ErrorKind::Internal, "unsigned"))?,
            )),
            _ => Arc::new(Int64Array::from(
                ints.iter()
                    .map(|&v| i64::try_from(v))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|_| Error::new(ErrorKind::Internal, "signed"))?,
            )),
        };
        fields.push(Field::new(&slug, value.data_type().clone(), true));
        columns.push(value);
        if has_time == Some(true) {
            let name = format!("{slug}_time");
            if micro == Some(true) {
                let us: Vec<Option<i64>> = times.iter().map(|t| t.map(|v| v / 1000)).collect();
                fields.push(Field::new(
                    name,
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                    true,
                ));
                columns.push(Arc::new(arrow_array::TimestampMicrosecondArray::from(us)));
            } else {
                fields.push(Field::new(
                    name,
                    DataType::Timestamp(TimeUnit::Nanosecond, None),
                    true,
                ));
                columns.push(Arc::new(TimestampNanosecondArray::from(times)));
            }
        }
        if !face {
            fields.push(Field::new("is_ghost_cell", DataType::Boolean, true));
            columns.push(Arc::new(BooleanArray::from(ghost)));
        }
        ArrowLayer::from_columns(
            &slug,
            fields,
            columns,
            ("mesh_name", Some(id_field)),
            options.batch_rows,
        )
    }

    /// Physical (non-ghost) cell count of a 2D area as ras-commander reads it; `None` when
    /// not recorded.
    pub fn physical_cell_count(
        &self,
        mesh: &str,
        metadata_max_bytes: usize,
    ) -> Result<Option<u64>> {
        self.mr_physical_cell_count(mesh, metadata_max_bytes)
    }
}

const NS_PER_DAY: i64 = 86_400_000_000_000;
const ROUND_NS: i64 = 100_000_000;

/// `start + pd.to_timedelta(days, unit="D")`, then `DatetimeIndex.round("100ms")`, exactly as
/// pandas computes it: the whole days and the fraction (rounded to 13 decimals, NumPy
/// `round`) are scaled separately and the fraction truncated to nanoseconds; then rounded
/// half to even to 100 ms. NaN gives null (NaT). Results outside the nanosecond range refuse.
pub fn timestamp_ns(start_ns: i64, days: f64) -> Result<Option<i64>> {
    if days.is_nan() {
        return Ok(None);
    }
    let range = || {
        refuse(
            UnsupportedCode::ResourceLimit,
            format!("{days} days outside the nanosecond range"),
        )
    };
    if !days.is_finite() || days.abs() >= 9.2e18 {
        return Err(range());
    }
    let base = days.trunc() as i64;
    let frac = days - base as f64;
    let scale = 1e13_f64;
    let frac = (frac * scale).round_ties_even() / scale;
    let delta = base
        .checked_mul(NS_PER_DAY)
        .and_then(|b| b.checked_add((frac * NS_PER_DAY as f64) as i64))
        .ok_or_else(range)?;
    let t = start_ns.checked_add(delta).ok_or_else(range)?;
    let (q, r) = (t.div_euclid(ROUND_NS), t.rem_euclid(ROUND_NS));
    let half = ROUND_NS / 2;
    let q = if r > half || (r == half && q % 2 != 0) {
        q + 1
    } else {
        q
    };
    q.checked_mul(ROUND_NS).map(Some).ok_or_else(range)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_names_and_aliases_are_unique() {
        let mut seen = HashSet::new();
        for d in CATALOG {
            assert!(seen.insert(d.name));
            for (_, a) in d.aliases {
                assert!(seen.insert(a));
            }
        }
        assert_eq!(lookup("mesh_areas").unwrap().name, "mesh_perimeters");
        assert_eq!(lookup("centerlines").unwrap().name, "river_centerlines");
        assert!(lookup("nope").is_none());
    }
    #[test]
    fn slugs_follow_ras_commander() {
        assert_eq!(
            variable_slug("Maximum Water Surface").unwrap(),
            "maximum_water_surface"
        );
        assert_eq!(
            variable_slug("Cell Last Iteration").unwrap(),
            "cell_last_iteration"
        );
        assert!(variable_slug("Tiefe ü").is_err());
    }
    #[test]
    fn plain_text_rule() {
        assert!(plain_ras_text("BaldEagleCr"));
        assert!(plain_ras_text("2D Area"));
        assert!(!plain_ras_text("True"));
        assert!(!plain_ras_text("01JAN2000 12:00:00"));
        assert!(!plain_ras_text("01JAN2000 1200"));
        assert!(!plain_ras_text("12:00:00"));
    }
    #[test]
    fn timestamps_round_half_even_to_100ms() {
        assert_eq!(timestamp_ns(0, f64::NAN).unwrap(), None);
        assert_eq!(timestamp_ns(0, 1.0).unwrap(), Some(NS_PER_DAY));
        // 0.05 s is exactly half of 100 ms: ties to the even multiple (0).
        assert_eq!(timestamp_ns(50_000_000, 0.0).unwrap(), Some(0));
        assert_eq!(timestamp_ns(150_000_000, 0.0).unwrap(), Some(200_000_000));
        assert_eq!(timestamp_ns(-50_000_000, 0.0).unwrap(), Some(0));
        assert!(timestamp_ns(0, 1e300).is_err());
    }
    #[test]
    fn wkb_matches_shapely_layout() {
        let mut w = Vec::new();
        wkb_line(&mut w, &[[0.0, 1.0], [2.0, 3.0]]).unwrap();
        assert_eq!(&w[..9], &[1, 2, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(w.len(), 9 + 32);
        let mut p = Vec::new();
        wkb_polygon(&mut p, &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.0, 0.0]]).unwrap();
        assert_eq!(&p[..13], &[1, 3, 0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0]);
    }
}
