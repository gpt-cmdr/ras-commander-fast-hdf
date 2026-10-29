//! Exact 1D cross-section geometry: ragged Info/Values tables per cross section (station-
//! elevation, Manning's n, ineffective areas, deck lines, hydraulic property tables), the exact
//! Attributes compound (reach lengths, banks, levees, coefficients), packed 1D line families
//! (cut lines, river centerlines/banks/edges/flow paths, structure centerlines, storage-area
//! polygons), river reach topology, junctions and cross-section interpolation surfaces.
//!
//! Nothing is recomputed: stored values keep their stored numeric type, Info rows keep their
//! stored integers, compounds keep their raw bytes. Absent objects are Unavailable, unproven
//! layouts Unsupported, contradictory storage Corrupt.
use super::mesh_geometry::{exact_compound, selected, ExactCompound};
use super::metadata::{
    typed_attribute, CompoundColumn, CompoundTable, MetadataValue, TypedAttribute,
};
use super::structures::ColumnLabel;
use super::xs1d::CrossSectionGeometry;
use super::{Identity, NumericArray, NumericKind, NumericValues, ReadOnlyHdfSession, Slice};
use crate::capability::error_to_capability;
use crate::{Capability, Error, ErrorKind, Reported, Result, UnsupportedCode};
use std::ops::Range;

const XS: &str = "Geometry/Cross Sections";
const XS_ATTRIBUTES: &str = "Geometry/Cross Sections/Attributes";
/// Separated-layout (HEC-RAS 5.x legacy or hybrid) anchors. Their presence is an unqualified
/// identity layout for these typed readers, never silently mixed with the compound layout.
const LEGACY_ANCHORS: [&str; 4] = ["Node Names", "River Stations", "River Names", "Reach Names"];

/// A qualified ragged per-feature Info/Values pair below `Geometry/Cross Sections`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum XsGeometryField {
    /// `Station Elevation Info` / `Station Elevation Values` (station, elevation).
    StationElevation,
    /// `Manning's n Info` / `Manning's n Values` (station, n).
    ManningsN,
    /// `Ineffective Info` / `Ineffective Blocks` (compound rows).
    IneffectiveAreas,
    /// `Deck High Info` / `Deck High Values` (HEC-RAS 5.x geometry).
    DeckHigh,
    /// `Deck Low Info` / `Deck Low Values` (HEC-RAS 5.x geometry).
    DeckLow,
    /// `Property Tables/XSEC Info` / `XSEC Value`: the per-section hydraulic property table
    /// (HTAB) read by `HdfHydraulicTables`. Info column 2 (`DS Cell`) is kept exactly.
    PropertyTable,
    /// `Property Tables/Cell Info` / `Cell Value`: per-cell (between sections) tables. Records
    /// are identified by their Cell Info row, not by a cross section.
    PropertyCells,
}
pub const XS_GEOMETRY_FIELDS: &[XsGeometryField] = &[
    XsGeometryField::StationElevation,
    XsGeometryField::ManningsN,
    XsGeometryField::IneffectiveAreas,
    XsGeometryField::DeckHigh,
    XsGeometryField::DeckLow,
    XsGeometryField::PropertyTable,
    XsGeometryField::PropertyCells,
];
impl XsGeometryField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StationElevation => "station_elevation",
            Self::ManningsN => "mannings_n",
            Self::IneffectiveAreas => "ineffective_areas",
            Self::DeckHigh => "deck_high",
            Self::DeckLow => "deck_low",
            Self::PropertyTable => "property_table",
            Self::PropertyCells => "property_cells",
        }
    }
    /// Info and Values dataset names relative to `Geometry/Cross Sections`.
    pub fn dataset_names(self) -> (&'static str, &'static str) {
        match self {
            Self::StationElevation => ("Station Elevation Info", "Station Elevation Values"),
            Self::ManningsN => ("Manning's n Info", "Manning's n Values"),
            Self::IneffectiveAreas => ("Ineffective Info", "Ineffective Blocks"),
            Self::DeckHigh => ("Deck High Info", "Deck High Values"),
            Self::DeckLow => ("Deck Low Info", "Deck Low Values"),
            Self::PropertyTable => ("Property Tables/XSEC Info", "Property Tables/XSEC Value"),
            Self::PropertyCells => ("Property Tables/Cell Info", "Property Tables/Cell Value"),
        }
    }
    fn compound(self) -> bool {
        self == Self::IneffectiveAreas
    }
    fn per_section(self) -> bool {
        self != Self::PropertyCells
    }
}
/// Descriptor of one qualified field. `capability` is the only status of the pair.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct XsGeometryDataset {
    pub field: XsGeometryField,
    pub info_path: String,
    pub values_path: String,
    pub info_shape: Vec<u64>,
    pub values_shape: Vec<u64>,
    pub info_kind: Option<NumericKind>,
    /// None for compound values (see `compound`).
    pub values_kind: Option<NumericKind>,
    pub compound: bool,
    /// Table whose rows identify the records that are not cross sections (records at or
    /// after `section_records`): `Geometry/Cross Sections/Attributes` when
    /// `records_are_sections`, else the Info dataset itself.
    pub identity_table: String,
    /// True only when every Info row is proven one-to-one with the cross-section Attributes
    /// rows (equal counts).
    pub records_are_sections: bool,
    /// Number of leading records identified as cross-section Attributes rows (record i is
    /// section i). Equals the section count when `records_are_sections`. For XSEC Info with
    /// more rows than sections (observed: one trailing extra row in HEC-RAS 6.6 plans that
    /// contain one inline structure; BaldEagle 179/178, Inline_3Gates 36/35, OrleansEx
    /// 336/335) the leading rows are assigned only when the first stored HTAB elevation of
    /// every row i equals the stored minimum station elevation of section i bit for bit;
    /// the trailing rows stay identified by their XSEC Info row. Otherwise 0 (conservative).
    pub section_records: u64,
    /// Value column labels from `Variables` ((n,2) name/unit) or `Column` (names) attributes.
    pub labels: Reported<Vec<ColumnLabel>>,
    pub info_attributes: Vec<TypedAttribute>,
    pub values_attributes: Vec<TypedAttribute>,
    pub capability: Capability,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct XsGeometryInventory {
    /// Validated identities (River/Reach/RS, name, banks) of the Attributes rows.
    pub geometry: CrossSectionGeometry,
    /// The full Attributes compound with exact raw bytes: reach lengths (`Len Left/Channel/
    /// Right`), banks, levees, contraction/expansion, HTAB parameters, friction mode, etc.
    pub attributes: ExactCompound,
    pub fields: Vec<XsGeometryDataset>,
    /// Datasets/groups below `Geometry/Cross Sections` that no qualified reader consumes
    /// (for example `Flow Distribution`, `Orthogonal Vectors`), each Unsupported.
    pub unqualified: Vec<(String, Capability)>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct XsGeometryRecord {
    /// Source, identity table and row of this record.
    pub id: Identity,
    /// Absolute stored rows of the Values dataset (Info columns 0/1).
    pub stored_rows: Range<u64>,
    /// First row of this record in the returned packed `values`.
    pub offset: u64,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum XsGeometryValues {
    Numeric(NumericArray),
    Compound(ExactCompound),
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct XsGeometrySelection {
    pub field: XsGeometryField,
    pub info_path: String,
    pub values_path: String,
    /// Exact stored Info rows of the selected records, in selection order.
    pub info: NumericArray,
    /// Selected Values rows packed in record order (repeated selections repeat rows).
    pub values: XsGeometryValues,
    pub records: Vec<XsGeometryRecord>,
    pub labels: Reported<Vec<ColumnLabel>>,
}

/// A qualified packed 1D line/polygon family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OneDLineFamily {
    CrossSectionCutLines,
    RiverCenterlines,
    RiverBankLines,
    RiverEdgeLines,
    RiverFlowPaths,
    StructureCenterlines,
    StorageAreaPolygons,
    /// `Geometry/Pipe Conduits`: storm and sewer pipe conduits (HEC-RAS 6.5+ pipe networks).
    /// Not part of [`ONE_D_LINE_FAMILIES`] (which lists the river and structure families); it
    /// is listed in [`PIPE_LINE_FAMILIES`].
    PipeConduits,
}
/// The pipe-network line families, read with the same packed-line rules as
/// [`ONE_D_LINE_FAMILIES`].
pub const PIPE_LINE_FAMILIES: &[OneDLineFamily] = &[OneDLineFamily::PipeConduits];
pub const ONE_D_LINE_FAMILIES: &[OneDLineFamily] = &[
    OneDLineFamily::CrossSectionCutLines,
    OneDLineFamily::RiverCenterlines,
    OneDLineFamily::RiverBankLines,
    OneDLineFamily::RiverEdgeLines,
    OneDLineFamily::RiverFlowPaths,
    OneDLineFamily::StructureCenterlines,
    OneDLineFamily::StorageAreaPolygons,
];
impl OneDLineFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CrossSectionCutLines => "cross_section_cut_lines",
            Self::RiverCenterlines => "river_centerlines",
            Self::RiverBankLines => "river_bank_lines",
            Self::RiverEdgeLines => "river_edge_lines",
            Self::RiverFlowPaths => "river_flow_paths",
            Self::StructureCenterlines => "structure_centerlines",
            Self::StorageAreaPolygons => "storage_area_polygons",
            Self::PipeConduits => "pipe_conduits",
        }
    }
    pub fn group_path(self) -> &'static str {
        match self {
            Self::CrossSectionCutLines => XS,
            Self::RiverCenterlines => "Geometry/River Centerlines",
            Self::RiverBankLines => "Geometry/River Bank Lines",
            Self::RiverEdgeLines => "Geometry/River Edge Lines",
            Self::RiverFlowPaths => "Geometry/River Flow Paths",
            Self::StructureCenterlines => "Geometry/Structures",
            Self::StorageAreaPolygons => "Geometry/Storage Areas",
            Self::PipeConduits => "Geometry/Pipe Conduits",
        }
    }
    /// Dataset prefix of the packed `Info`/`Parts`/`Points` triple.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::RiverFlowPaths => "Flow Path Lines",
            Self::StructureCenterlines => "Centerline",
            Self::StorageAreaPolygons => "Polygon",
            _ => "Polyline",
        }
    }
    fn attributes_required(self) -> bool {
        matches!(
            self,
            Self::CrossSectionCutLines
                | Self::RiverCenterlines
                | Self::StructureCenterlines
                | Self::StorageAreaPolygons
                | Self::PipeConduits
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OneDLineFeature {
    /// Attributes row when the family has an Attributes table, else the Info row.
    pub id: Identity,
    pub point_range: Range<u64>,
    /// Absolute ranges into `points`; stored Parts offsets are relative to `point_range`.
    pub parts: Vec<Range<u64>>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OneDLineData {
    pub family: OneDLineFamily,
    pub attributes: Option<ExactCompound>,
    pub info: NumericArray,
    pub parts: NumericArray,
    pub points: NumericArray,
    pub features: Vec<OneDLineFeature>,
    /// Typed attributes of the Points dataset (for example `Column` = X, Y).
    pub points_attributes: Vec<TypedAttribute>,
}
/// One `Geometry/River Centerlines/Attributes` row: the stored reach topology.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RiverReach {
    pub id: Identity,
    pub river: String,
    pub reach: String,
    pub upstream_type: Reported<String>,
    pub upstream_name: Reported<String>,
    pub downstream_type: Reported<String>,
    pub downstream_name: Reported<String>,
}
/// A point family with an Attributes compound and one stored point per row.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OneDPointData {
    pub group: String,
    pub attributes: ExactCompound,
    pub points: NumericArray,
    pub features: Vec<Identity>,
    pub names: Vec<Reported<String>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InterpolationSurface {
    /// Row of `TIN Info`.
    pub id: Identity,
    pub point_range: Range<u64>,
    pub triangle_range: Range<u64>,
}
/// `Geometry/Cross Section Interpolation Surfaces`, exact. Triangle vertex indices are kept as
/// stored; their base (surface-local or global) is not asserted.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct XsInterpolationSurfaces {
    pub info: NumericArray,
    pub points: NumericArray,
    pub triangles: NumericArray,
    /// Optional per-surface arrays (`XSIDs`, `Areas`, `Orthogonal Vectors`, `Bounding Lines`).
    pub per_surface: Vec<(String, NumericArray)>,
    pub surfaces: Vec<InterpolationSurface>,
}

pub(super) fn invalid(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidRequest, m)
}
pub(super) fn corrupt(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt, m)
}
pub(super) fn unsupported(code: UnsupportedCode, m: impl Into<String>) -> Error {
    Error::unsupported(code, m)
}
pub(super) fn width(k: NumericKind) -> u64 {
    match k {
        NumericKind::I8 | NumericKind::U8 => 1,
        NumericKind::I16 | NumericKind::U16 => 2,
        NumericKind::I32 | NumericKind::U32 | NumericKind::F32 => 4,
        _ => 8,
    }
}
pub(super) fn budget(bytes: u64, max_bytes: usize) -> Result<()> {
    if bytes > max_bytes as u64 {
        Err(invalid("selected data exceeds caller byte limit")
            .with_code(UnsupportedCode::ResourceLimit))
    } else {
        Ok(())
    }
}
/// Gather row ranges `(start..end)` of a row-major array with `cols` columns into a packed copy.
pub(super) fn take_rows(
    values: &NumericValues,
    cols: usize,
    ranges: &[Range<usize>],
) -> NumericValues {
    macro_rules! take {
        ($($v:ident),*) => {
            match values {
                $(NumericValues::$v(v) => {
                    let mut out = Vec::new();
                    for r in ranges {
                        out.extend_from_slice(&v[r.start * cols..r.end * cols]);
                    }
                    NumericValues::$v(out)
                })*
            }
        };
    }
    take!(I8, I16, I32, I64, U8, U16, U32, U64, F32, F64)
}
/// Stored integers of an Info-like array as exact i64 (only proven integer kinds).
pub(super) fn integers(a: &NumericArray) -> Result<Vec<i64>> {
    Ok(match &a.values {
        NumericValues::I32(v) => v.iter().map(|&x| i64::from(x)).collect(),
        NumericValues::I64(v) => v.clone(),
        NumericValues::I16(v) => v.iter().map(|&x| i64::from(x)).collect(),
        NumericValues::U32(v) => v.iter().map(|&x| i64::from(x)).collect(),
        NumericValues::U16(v) => v.iter().map(|&x| i64::from(x)).collect(),
        _ => {
            return Err(unsupported(
                UnsupportedCode::Encoding,
                "index table requires proven integer storage",
            ))
        }
    })
}
/// Validated `start..start+count` inside `bound`.
pub(super) fn checked_range(start: i64, count: i64, bound: u64) -> Result<Range<u64>> {
    let start = u64::try_from(start).map_err(|_| corrupt("negative stored start index"))?;
    let count = u64::try_from(count).map_err(|_| corrupt("negative stored count"))?;
    let end = start
        .checked_add(count)
        .filter(|&e| e <= bound)
        .ok_or_else(|| corrupt("stored index range outside the values array"))?;
    Ok(start..end)
}
/// Every attribute of a dataset, typed, with each payload bounded by `max_bytes`.
pub(super) fn dataset_typed_attributes(
    s: &ReadOnlyHdfSession,
    path: &str,
    max_bytes: usize,
) -> Result<Vec<TypedAttribute>> {
    let ds = s.file.dataset(path).map_err(super::encoding::map_error)?;
    ds.raw_attributes()
        .map_err(super::encoding::map_error)?
        .iter()
        .map(|a| {
            budget(a.message().raw_data.len() as u64, max_bytes)?;
            typed_attribute(a)
        })
        .collect()
}
/// Column labels declared by `Variables` / `Variable_Unit` ((n,2) name/unit) or `Column` /
/// `Column=` (n names). Several label attributes, or a count mismatch, are Corrupt.
pub(super) fn declared_labels(
    attrs: &[TypedAttribute],
    columns: u64,
) -> Reported<Vec<ColumnLabel>> {
    let found: Vec<&TypedAttribute> = attrs
        .iter()
        .filter(|a| {
            matches!(
                a.name.as_str(),
                "Variables" | "Variable_Unit" | "Column" | "Column="
            )
        })
        .collect();
    let Some(a) = found.first() else {
        return Reported::unavailable("no column-label attribute");
    };
    let raw = a.raw_bytes.clone();
    if found.len() > 1 {
        return Reported::corrupt("several column-label attributes").with_raw_bytes(raw);
    }
    let pairs = matches!(a.name.as_str(), "Variables" | "Variable_Unit");
    match &a.value {
        MetadataValue::Strings(v) if pairs && a.shape == [columns, 2] => Reported::supported(
            v.as_chunks::<2>()
                .0
                .iter()
                .map(|p| ColumnLabel {
                    name: p[0].clone(),
                    unit: Some(p[1].clone()),
                })
                .collect(),
        )
        .with_raw_bytes(raw),
        MetadataValue::Strings(v) if !pairs && a.shape == [columns] => Reported::supported(
            v.iter()
                .map(|n| ColumnLabel {
                    name: n.clone(),
                    unit: None,
                })
                .collect(),
        )
        .with_raw_bytes(raw),
        MetadataValue::Strings(_) => Reported::corrupt(format!(
            "{} shape {:?} does not match {columns} columns",
            a.name, a.shape
        ))
        .with_raw_bytes(raw),
        MetadataValue::Unsupported { code, reason } => {
            Reported::unsupported(*code, reason.clone()).with_raw_bytes(raw)
        }
        MetadataValue::Corrupt { reason } => Reported::corrupt(reason.clone()).with_raw_bytes(raw),
        _ => Reported::unsupported(
            UnsupportedCode::Encoding,
            "labels are not proven fixed text",
        )
        .with_raw_bytes(raw),
    }
}
/// Row subset of an exact compound (raw rows and decoded columns), in the given order.
pub(super) fn subset_compound(c: &ExactCompound, rows: &[Range<usize>]) -> Result<ExactCompound> {
    let n = usize::try_from(c.table.shape.first().copied().unwrap_or(0))
        .map_err(|_| corrupt("compound row count overflow"))?;
    let width = c.raw_bytes.len().checked_div(n).unwrap_or(0);
    if width * n != c.raw_bytes.len() {
        return Err(corrupt("compound payload is not a whole number of rows"));
    }
    let mut raw = Vec::new();
    let mut count = 0u64;
    for r in rows {
        if r.end > n {
            return Err(corrupt("compound row outside the table"));
        }
        raw.extend_from_slice(&c.raw_bytes[r.start * width..r.end * width]);
        count += (r.end - r.start) as u64;
    }
    let pick = |v: &MetadataValue| -> MetadataValue {
        macro_rules! pick {
            ($x:expr, $variant:ident) => {{
                let mut out = Vec::new();
                for r in rows {
                    out.extend_from_slice(&$x[r.start..r.end.min($x.len())]);
                }
                MetadataValue::$variant(out)
            }};
        }
        match v {
            MetadataValue::Signed(x) => pick!(x, Signed),
            MetadataValue::Unsigned(x) => pick!(x, Unsigned),
            MetadataValue::Float(x) => pick!(x, Float),
            MetadataValue::Strings(x) => pick!(x, Strings),
            other => other.clone(),
        }
    };
    Ok(ExactCompound {
        table: CompoundTable {
            shape: vec![count],
            dataspace: crate::hdf::Dataspace::Simple(vec![count]),
            columns: c
                .table
                .columns
                .iter()
                .map(|col| CompoundColumn {
                    name: col.name.clone(),
                    byte_offset: col.byte_offset,
                    attribute_type: col.attribute_type.clone(),
                    datatype_label: col.datatype_label.clone(),
                    value: pick(&col.value),
                })
                .collect(),
        },
        raw_bytes: raw,
    })
}
/// Text column of a compound, exactly as decoded.
pub(super) fn text_column<'a>(t: &'a CompoundTable, name: &str) -> Option<&'a CompoundColumn> {
    t.columns.iter().find(|c| c.name == name)
}
pub(super) fn reported_text(t: &CompoundTable, name: &str, row: usize) -> Result<Reported<String>> {
    Ok(match text_column(t, name) {
        None => Reported::unavailable(format!("column {name} absent")),
        Some(c) => match &c.value {
            MetadataValue::Strings(v) => Reported::supported(
                v.get(row)
                    .ok_or_else(|| corrupt("compound column length mismatch"))?
                    .clone(),
            ),
            MetadataValue::Unsupported { code, reason } => {
                Reported::unsupported(*code, reason.clone())
            }
            MetadataValue::Corrupt { reason } => Reported::corrupt(reason.clone()),
            _ => Reported::unsupported(
                UnsupportedCode::Encoding,
                format!("{name} is not proven fixed text"),
            ),
        },
    })
}
/// Full exact numeric dataset with its stored kind, bounded by `max_bytes`.
pub(super) fn exact_array(
    s: &ReadOnlyHdfSession,
    path: &str,
    max_bytes: usize,
) -> Result<NumericArray> {
    selected(s, path, None, max_bytes)
}

impl ReadOnlyHdfSession {
    fn xs_layout_guard(&self) -> Result<()> {
        let members = self.group_members(XS)?;
        if let Some(a) = LEGACY_ANCHORS
            .iter()
            .find(|a| members.datasets.iter().any(|d| d == *a))
        {
            return Err(unsupported(
                UnsupportedCode::IdentityLayout,
                format!(
                    "separated (legacy/hybrid) cross-section layout dataset {a} is unqualified"
                ),
            ));
        }
        Ok(())
    }
    fn xs_field_probe(
        &self,
        field: XsGeometryField,
        sections: u64,
        max_bytes: usize,
    ) -> Result<XsGeometryDataset> {
        let (info_name, values_name) = field.dataset_names();
        let info_path = format!("{XS}/{info_name}");
        let values_path = format!("{XS}/{values_name}");
        let mut d = XsGeometryDataset {
            field,
            info_path: info_path.clone(),
            values_path: values_path.clone(),
            info_shape: vec![],
            values_shape: vec![],
            info_kind: None,
            values_kind: None,
            compound: field.compound(),
            identity_table: if field.per_section() {
                XS_ATTRIBUTES.into()
            } else {
                info_path.clone()
            },
            records_are_sections: field.per_section(),
            section_records: if field.per_section() { sections } else { 0 },
            labels: Reported::unavailable("field absent"),
            info_attributes: vec![],
            values_attributes: vec![],
            capability: Capability::Supported,
        };
        let probe = (|| -> Result<Option<()>> {
            let has_info = self.has_dataset(&info_path)?;
            let has_values = self.has_dataset(&values_path)?;
            match (has_info, has_values) {
                (false, false) => return Ok(None),
                (true, true) => {}
                _ => return Err(corrupt("incomplete Info/Values dataset pair")),
            }
            let ic = self.numeric_capability(&info_path)?;
            d.info_shape = ic.shape.clone();
            d.info_kind = ic.kind;
            d.info_attributes = dataset_typed_attributes(self, &info_path, max_bytes)?;
            if !matches!(ic.kind, Some(NumericKind::I32 | NumericKind::I64)) {
                return Err(unsupported(
                    UnsupportedCode::Encoding,
                    "Info table requires proven i32/i64 storage",
                ));
            }
            if ic.shape.len() != 2 || ic.shape[1] < 2 {
                return Err(corrupt("Info table must be (rows, >=2)"));
            }
            if field == XsGeometryField::PropertyTable && ic.shape[0] != sections {
                d.identity_table = info_path.clone();
                d.records_are_sections = false;
                d.section_records = 0;
            } else if field.per_section() && ic.shape[0] != sections {
                return Err(corrupt(format!(
                    "{info_path}: {} rows but {sections} cross sections",
                    ic.shape[0]
                )));
            }
            d.values_attributes = dataset_typed_attributes(self, &values_path, max_bytes)?;
            let bound = if field.compound() {
                let ds = self
                    .file
                    .dataset(&values_path)
                    .map_err(super::encoding::map_error)?;
                d.values_shape = ds.shape().to_vec();
                if d.values_shape.len() != 1 {
                    return Err(corrupt("compound Values must be rank one"));
                }
                // Prove the compound layout (and budget) without retaining it.
                exact_compound(self, &values_path, max_bytes)?;
                d.labels = Reported::unavailable("compound values carry member names");
                d.values_shape[0]
            } else {
                let vc = self.numeric_capability(&values_path)?;
                d.values_shape = vc.shape.clone();
                d.values_kind = vc.kind;
                if !matches!(vc.kind, Some(NumericKind::F32 | NumericKind::F64)) {
                    return Err(unsupported(
                        UnsupportedCode::Encoding,
                        "Values require proven f32/f64 storage",
                    ));
                }
                if vc.shape.len() != 2 || vc.shape[1] == 0 {
                    return Err(corrupt("Values must be (rows, columns)"));
                }
                d.labels = declared_labels(&d.values_attributes, vc.shape[1]);
                vc.shape[0]
            };
            let info = exact_array(self, &info_path, max_bytes)?;
            let iv = integers(&info)?;
            let cols = ic.shape[1] as usize;
            for row in iv.chunks_exact(cols) {
                checked_range(row[0], row[1], bound)?;
            }
            // The optional HTAB identity count of a property table with more Info
            // rows than sections is added by `xs_geometry_inventory` (see there).
            Ok(Some(()))
        })();
        match probe {
            Ok(Some(())) => {}
            Ok(None) => d.capability = Capability::unavailable("canonical Info/Values pair absent"),
            Err(e) => d.capability = error_to_capability(e)?,
        }
        Ok(d)
    }
    /// Leading XSEC Info rows qualified as sections 0..n by exact first-elevation identity
    /// (first HTAB elevation == minimum stored station elevation of the section). Any read or
    /// layout problem, or any mismatch, qualifies nothing.
    fn htab_leading_sections(
        &self,
        info: &[i64],
        cols: usize,
        values_path: &str,
        sections: u64,
        max_bytes: usize,
    ) -> Result<u64> {
        let (si, sv) = XsGeometryField::StationElevation.dataset_names();
        let se_info = integers(&exact_array(self, &format!("{XS}/{si}"), max_bytes)?)?;
        let se = exact_array(self, &format!("{XS}/{sv}"), max_bytes)?;
        let htab = exact_array(self, values_path, max_bytes)?;
        let as_f64 = |a: &NumericArray| -> Option<Vec<f64>> {
            match &a.values {
                NumericValues::F32(v) => Some(v.iter().map(|&x| f64::from(x)).collect()),
                NumericValues::F64(v) => Some(v.clone()),
                _ => None,
            }
        };
        let (Some(sev), Some(hv)) = (as_f64(&se), as_f64(&htab)) else {
            return Ok(0);
        };
        // This optional identity probe reads station elevations independently
        // of their field descriptor, so it must prove their rank itself.
        let ([_, sec], [_, hc]) = (se.shape.as_slice(), htab.shape.as_slice()) else {
            return Err(corrupt("HTAB identity evidence must be rank two"));
        };
        let (sec, hc) = (*sec as usize, *hc as usize);
        if hc == 0 {
            return Err(corrupt("HTAB identity evidence has no columns"));
        }
        if se_info.len() != sections as usize * 2 || sec < 2 {
            return Ok(0);
        }
        for i in 0..sections as usize {
            let r = checked_range(se_info[i * 2], se_info[i * 2 + 1], se.shape[0])?;
            let min = (r.start..r.end)
                .map(|k| sev[k as usize * sec + 1])
                .fold(f64::INFINITY, f64::min);
            let start = usize::try_from(info[i * cols]).map_err(|_| corrupt("negative start"))?;
            if r.is_empty() || info[i * cols + 1] < 1 || hv[start * hc].to_bits() != min.to_bits() {
                return Ok(0);
            }
        }
        Ok(sections)
    }
    /// Identities, the exact Attributes compound and every qualified ragged field with its own
    /// capability. `max_bytes` bounds each consulted dataset/attribute. Separated (5.x legacy or
    /// hybrid) layouts are Unsupported(identity_layout); absence of the XS table is NotFound.
    pub fn xs_geometry_inventory(&self, max_bytes: usize) -> Result<XsGeometryInventory> {
        self.memo(format!("xs_geometry_inventory:{max_bytes}"), || {
            let mut inv = self.xs_geometry_inventory_core(max_bytes)?;
            let sections = inv.geometry.sections.len() as u64;
            for d in &mut inv.fields {
                // A property table with more Info rows than sections: qualify its leading
                // rows as sections by HTAB identity. Any problem qualifies none (0).
                if d.field == XsGeometryField::PropertyTable
                    && matches!(d.capability, Capability::Supported)
                    && d.info_shape.first().is_some_and(|&rows| rows > sections)
                {
                    let iv = integers(&exact_array(self, &d.info_path, max_bytes)?)?;
                    let cols = d.info_shape[1] as usize;
                    d.section_records = self
                        .htab_leading_sections(&iv, cols, &d.values_path, sections, max_bytes)
                        .unwrap_or(0);
                }
            }
            Ok(inv)
        })
    }
    /// [`xs_geometry_inventory`](Self::xs_geometry_inventory) without the property table's
    /// HTAB identity count (`section_records` of a property table with more Info rows than
    /// sections stays 0). Every other descriptor is identical. The count decodes the whole
    /// HTAB value table, which HEC-RAS stores with one row per compressed chunk; reads of
    /// the other fields do not depend on it, so they use this inventory.
    fn xs_geometry_inventory_core(&self, max_bytes: usize) -> Result<XsGeometryInventory> {
        self.memo(format!("xs_geometry_inventory_core:{max_bytes}"), || {
            self.xs_layout_guard()?;
            let geometry = self.cross_sections(max_bytes)?;
            let attributes = exact_compound(self, XS_ATTRIBUTES, max_bytes)?;
            let n = geometry.sections.len() as u64;
            let mut fields = Vec::new();
            for &f in XS_GEOMETRY_FIELDS {
                fields.push(self.xs_field_probe(f, n, max_bytes)?);
            }
            let mut consumed: Vec<String> = vec!["Attributes".into()];
            for f in XS_GEOMETRY_FIELDS {
                let (i, v) = f.dataset_names();
                consumed.push(i.into());
                consumed.push(v.into());
            }
            for p in ["Polyline Info", "Polyline Parts", "Polyline Points"] {
                consumed.push(p.into());
            }
            let mut unqualified = Vec::new();
            let mut pending = vec![(XS.to_string(), String::new(), 0usize)];
            while let Some((group, rel, depth)) = pending.pop() {
                let m = self.group_members(&group)?;
                for d in m.datasets {
                    let r = if rel.is_empty() {
                        d.clone()
                    } else {
                        format!("{rel}/{d}")
                    };
                    if !consumed.contains(&r) {
                        unqualified.push((
                            format!("{XS}/{r}"),
                            Capability::unsupported(
                                UnsupportedCode::NotImplemented,
                                "no qualified typed reader for this cross-section dataset",
                            ),
                        ));
                    }
                }
                for g in m.groups {
                    let r = if rel.is_empty() {
                        g.clone()
                    } else {
                        format!("{rel}/{g}")
                    };
                    if depth >= 3 {
                        return Err(unsupported(
                            UnsupportedCode::ResourceLimit,
                            "cross-section group nesting limit",
                        ));
                    }
                    pending.push((format!("{group}/{g}"), r, depth + 1));
                }
            }
            unqualified.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(XsGeometryInventory {
                geometry,
                attributes,
                fields,
                unqualified,
            })
        })
    }
    /// Exact ragged rows for selected records (`None` = all, in stored order). Records are
    /// cross-section rows (Cell Info rows for `PropertyCells`); unsorted and repeated
    /// selections are kept. `metadata_max_bytes` bounds each consulted metadata dataset;
    /// `max_bytes` bounds the returned Info + Values bytes (compound Values: the stored table).
    pub fn read_xs_geometry(
        &self,
        field: XsGeometryField,
        records: Option<&[u64]>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<XsGeometrySelection> {
        // Only the requested field's descriptor is used; the HTAB identity count is
        // needed only by the property table itself.
        let inv = if field == XsGeometryField::PropertyTable {
            self.xs_geometry_inventory(metadata_max_bytes)?
        } else {
            self.xs_geometry_inventory_core(metadata_max_bytes)?
        };
        let d = inv
            .fields
            .iter()
            .find(|d| d.field == field)
            .ok_or_else(|| invalid("unknown cross-section field"))?;
        match &d.capability {
            Capability::Supported => {}
            Capability::Unavailable { reason } => {
                return Err(Error::new(ErrorKind::NotFound, reason.clone()))
            }
            Capability::Unsupported { code, reason } => {
                return Err(unsupported(*code, reason.clone()))
            }
            c => return Err(corrupt(c.reason().unwrap_or("invalid cross-section field"))),
        }
        let info = exact_array(self, &d.info_path, metadata_max_bytes)?;
        let icols = d.info_shape[1] as usize;
        let n = d.info_shape[0];
        let selection: Vec<u64> = match records {
            Some(r) => {
                if let Some(bad) = r.iter().find(|&&i| i >= n) {
                    return Err(invalid(format!("record {bad} out of bounds ({n} records)")));
                }
                r.to_vec()
            }
            None => (0..n).collect(),
        };
        let iv = integers(&info)?;
        let bound = d.values_shape[0];
        let mut out = Vec::with_capacity(selection.len());
        let mut ranges = Vec::with_capacity(selection.len());
        let mut offset = 0u64;
        for &i in &selection {
            let row = &iv[i as usize * icols..i as usize * icols + icols];
            let stored = checked_range(row[0], row[1], bound)?;
            ranges.push(stored.start as usize..stored.end as usize);
            out.push(XsGeometryRecord {
                id: if i < d.section_records {
                    Identity::new(self.path(), XS_ATTRIBUTES, i)
                } else {
                    Identity::new(self.path(), &d.identity_table, i)
                },
                stored_rows: stored.clone(),
                offset,
            });
            offset += stored.end - stored.start;
        }
        let info_rows: Vec<Range<usize>> = selection
            .iter()
            .map(|&i| i as usize..i as usize + 1)
            .collect();
        let info_bytes =
            selection.len() as u64 * icols as u64 * width(d.info_kind.unwrap_or(NumericKind::I32));
        let sel_info = NumericArray {
            shape: vec![selection.len() as u64, icols as u64],
            values: take_rows(&info.values, icols, &info_rows),
        };
        let values = if d.compound {
            let table = exact_compound(self, &d.values_path, max_bytes)?;
            budget(info_bytes + table.raw_bytes.len() as u64, max_bytes)?;
            XsGeometryValues::Compound(subset_compound(&table, &ranges)?)
        } else {
            let vcols = d.values_shape[1];
            let kind = d
                .values_kind
                .ok_or_else(|| unsupported(UnsupportedCode::Encoding, "unproven values"))?;
            let returned = offset
                .checked_mul(vcols)
                .and_then(|x| x.checked_mul(width(kind)))
                .ok_or_else(|| invalid("selection size overflow"))?;
            budget(returned + info_bytes, max_bytes)?;
            let lo = ranges.iter().map(|r| r.start).min().unwrap_or(0) as u64;
            let hi = ranges.iter().map(|r| r.end).max().unwrap_or(0) as u64;
            let span = (hi - lo) * vcols * width(kind);
            let packed = if offset == 0 {
                concat(Vec::new(), kind)
            } else if returned + info_bytes + span <= max_bytes as u64 {
                let a = selected(self, &d.values_path, Some(Slice::new(lo, hi, 1)), max_bytes)?;
                let rel: Vec<Range<usize>> = ranges
                    .iter()
                    .map(|r| r.start - lo as usize..r.end - lo as usize)
                    .collect();
                take_rows(&a.values, vcols as usize, &rel)
            } else {
                // Sparse selection: read each record separately (no retained intermediate).
                let mut parts: Vec<NumericValues> = Vec::new();
                for r in &ranges {
                    if r.is_empty() {
                        continue;
                    }
                    let a = selected(
                        self,
                        &d.values_path,
                        Some(Slice::new(r.start as u64, r.end as u64, 1)),
                        max_bytes,
                    )?;
                    parts.push(a.values);
                }
                concat(parts, kind)
            };
            XsGeometryValues::Numeric(NumericArray {
                shape: vec![offset, vcols],
                values: packed,
            })
        };
        Ok(XsGeometrySelection {
            field,
            info_path: d.info_path.clone(),
            values_path: d.values_path.clone(),
            info: sel_info,
            values,
            records: out,
            labels: d.labels.clone(),
        })
    }
    /// One packed 1D line/polygon family with validated feature ranges. Absent family:
    /// Unavailable. `max_bytes` bounds each dataset.
    pub fn one_d_line_family(
        &self,
        family: OneDLineFamily,
        max_bytes: usize,
    ) -> Result<Reported<OneDLineData>> {
        // Canonical absence, then every dataset read below (see `presence`).
        if let Some(reason) = self.one_d_line_prelude(family)? {
            return Ok(Reported::unavailable(reason));
        }
        let group = family.group_path();
        let prefix = family.prefix();
        let info_path = format!("{group}/{prefix} Info");
        if family == OneDLineFamily::CrossSectionCutLines {
            self.xs_layout_guard()?;
        }
        let attr_path = format!("{group}/Attributes");
        let attributes = if self.has_dataset(&attr_path)? {
            Some(exact_compound(self, &attr_path, max_bytes)?)
        } else if family.attributes_required() {
            return Err(unsupported(
                UnsupportedCode::IdentityLayout,
                "identity Attributes table absent",
            ));
        } else {
            None
        };
        let info = exact_array(self, &info_path, max_bytes)?;
        let parts = exact_array(self, &format!("{group}/{prefix} Parts"), max_bytes)?;
        let points_path = format!("{group}/{prefix} Points");
        let points = exact_array(self, &points_path, max_bytes)?;
        let points_attributes = dataset_typed_attributes(self, &points_path, max_bytes)?;
        if info.shape.len() != 2
            || info.shape[1] != 4
            || parts.shape.len() != 2
            || parts.shape[1] != 2
            || points.shape.len() != 2
            || points.shape[1] < 2
        {
            return Err(corrupt("invalid packed line Info/Parts/Points shape"));
        }
        if !matches!(points.values, NumericValues::F32(_) | NumericValues::F64(_)) {
            return Err(unsupported(
                UnsupportedCode::Encoding,
                "line coordinates must be f32/f64",
            ));
        }
        if let Some(a) = &attributes {
            if a.table.shape != [info.shape[0]] {
                return Err(corrupt("Attributes and Info row counts differ"));
            }
        }
        let iv = integers(&info)?;
        let pv = integers(&parts)?;
        let table = if attributes.is_some() {
            attr_path
        } else {
            info_path
        };
        let mut features = Vec::new();
        for (row, i) in iv.as_chunks::<4>().0.iter().enumerate() {
            let point_range = checked_range(i[0], i[1], points.shape[0])?;
            let part_range = checked_range(i[2], i[3], parts.shape[0])?;
            let mut ranges = Vec::new();
            for p in part_range {
                let p = p as usize;
                let r = checked_range(
                    pv[p * 2],
                    pv[p * 2 + 1],
                    point_range.end - point_range.start,
                )?;
                ranges.push(point_range.start + r.start..point_range.start + r.end);
            }
            features.push(OneDLineFeature {
                id: Identity::new(self.path(), &table, row as u64),
                point_range,
                parts: ranges,
            });
        }
        Ok(Reported::supported(OneDLineData {
            family,
            attributes,
            info,
            parts,
            points,
            features,
            points_attributes,
        }))
    }
    /// Stored reach topology of `Geometry/River Centerlines/Attributes`: exact River/Reach
    /// names and the upstream/downstream type and name of each reach (junction names as
    /// stored). Absent: Unavailable. No connectivity is inferred.
    pub fn river_reaches(&self, max_bytes: usize) -> Result<Reported<Vec<RiverReach>>> {
        let path = "Geometry/River Centerlines/Attributes";
        if !self.has_dataset(path)? {
            return Ok(Reported::unavailable("river centerline attributes absent"));
        }
        let t = exact_compound(self, path, max_bytes)?.table;
        if t.shape.len() != 1 {
            return Err(corrupt("river centerline attributes must be rank one"));
        }
        let mut out = Vec::new();
        for row in 0..t.shape[0] as usize {
            let required = |name: &str| -> Result<String> {
                let r = reported_text(&t, name, row)?;
                match r.value() {
                    Some(v) if !v.trim().is_empty() => Ok(v.clone()),
                    Some(_) => Err(corrupt(format!("empty reach identity {name}"))),
                    None => Err(unsupported(
                        UnsupportedCode::IdentityLayout,
                        format!("reach identity field {name} unproven or absent"),
                    )),
                }
            };
            out.push(RiverReach {
                id: Identity::new(self.path(), path, row as u64),
                river: required("River Name")?,
                reach: required("Reach Name")?,
                upstream_type: reported_text(&t, "US Type", row)?,
                upstream_name: reported_text(&t, "US Name", row)?,
                downstream_type: reported_text(&t, "DS Type", row)?,
                downstream_name: reported_text(&t, "DS Name", row)?,
            });
        }
        let mut seen = std::collections::HashSet::new();
        for r in &out {
            if !seen.insert((r.river.trim().to_string(), r.reach.trim().to_string())) {
                return Err(corrupt("duplicate trimmed river/reach identity"));
            }
        }
        Ok(Reported::supported(out))
    }
    pub(super) fn point_family(
        &self,
        group: &str,
        max_bytes: usize,
    ) -> Result<Reported<OneDPointData>> {
        if let Some(reason) = self.point_family_prelude(group)? {
            return Ok(Reported::unavailable(reason));
        }
        let attr_path = format!("{group}/Attributes");
        let points_path = format!("{group}/Points");
        let attributes = exact_compound(self, &attr_path, max_bytes)?;
        let points = exact_array(self, &points_path, max_bytes)?;
        if points.shape.len() != 2
            || points.shape[1] != 2
            || attributes.table.shape != [points.shape[0]]
        {
            return Err(corrupt("point family Attributes/Points shape mismatch"));
        }
        if !matches!(points.values, NumericValues::F32(_) | NumericValues::F64(_)) {
            return Err(unsupported(
                UnsupportedCode::Encoding,
                "points must be f32/f64",
            ));
        }
        let n = points.shape[0];
        let mut names = Vec::new();
        for row in 0..n as usize {
            names.push(reported_text(&attributes.table, "Name", row)?);
        }
        Ok(Reported::supported(OneDPointData {
            group: group.into(),
            features: (0..n)
                .map(|r| Identity::new(self.path(), &attr_path, r))
                .collect(),
            attributes,
            points,
            names,
        }))
    }
    /// `Geometry/Pipe Nodes` Attributes compound and stored points, one per row (manholes,
    /// junctions and outfalls of a pipe network). Same rules as [`Self::river_junctions`].
    pub fn pipe_nodes(&self, max_bytes: usize) -> Result<Reported<OneDPointData>> {
        self.point_family("Geometry/Pipe Nodes", max_bytes)
    }
    /// `Geometry/Junctions` Attributes compound and stored points, one per row.
    pub fn river_junctions(&self, max_bytes: usize) -> Result<Reported<OneDPointData>> {
        self.point_family("Geometry/Junctions", max_bytes)
    }
    /// Exact interpolation-surface TINs with validated point/triangle ranges per surface.
    pub fn xs_interpolation_surfaces(
        &self,
        max_bytes: usize,
    ) -> Result<Reported<XsInterpolationSurfaces>> {
        let group = "Geometry/Cross Section Interpolation Surfaces";
        if !self.has_group(group)? {
            return Ok(Reported::unavailable("interpolation surfaces absent"));
        }
        let info_path = format!("{group}/TIN Info");
        if !self.has_dataset(&info_path)? {
            return Ok(Reported::unavailable("TIN Info absent"));
        }
        let info = exact_array(self, &info_path, max_bytes)?;
        let points = exact_array(self, &format!("{group}/TIN Points"), max_bytes)?;
        let triangles = exact_array(self, &format!("{group}/TIN Triangles"), max_bytes)?;
        if info.shape.len() != 2
            || info.shape[1] != 4
            || points.shape.len() != 2
            || triangles.shape.len() != 2
            || triangles.shape[1] != 3
        {
            return Err(corrupt("invalid TIN Info/Points/Triangles shape"));
        }
        let iv = integers(&info)?;
        integers(&triangles)?;
        let mut surfaces = Vec::new();
        for (row, i) in iv.as_chunks::<4>().0.iter().enumerate() {
            surfaces.push(InterpolationSurface {
                id: Identity::new(self.path(), &info_path, row as u64),
                point_range: checked_range(i[0], i[1], points.shape[0])?,
                triangle_range: checked_range(i[2], i[3], triangles.shape[0])?,
            });
        }
        let mut per_surface = Vec::new();
        for name in ["XSIDs", "Areas", "Orthogonal Vectors", "Bounding Lines"] {
            let p = format!("{group}/{name}");
            if self.has_dataset(&p)? {
                let a = exact_array(self, &p, max_bytes)?;
                if a.shape.first() != Some(&info.shape[0]) {
                    return Err(corrupt(format!("{p} row count differs from TIN Info")));
                }
                per_surface.push((name.to_string(), a));
            }
        }
        Ok(Reported::supported(XsInterpolationSurfaces {
            info,
            points,
            triangles,
            per_surface,
            surfaces,
        }))
    }
}
pub(super) fn concat(parts: Vec<NumericValues>, kind: NumericKind) -> NumericValues {
    macro_rules! cat {
        ($v:ident) => {{
            let mut out = Vec::new();
            for p in parts {
                if let NumericValues::$v(x) = p {
                    out.extend(x);
                }
            }
            NumericValues::$v(out)
        }};
    }
    match kind {
        NumericKind::I8 => cat!(I8),
        NumericKind::I16 => cat!(I16),
        NumericKind::I32 => cat!(I32),
        NumericKind::I64 => cat!(I64),
        NumericKind::U8 => cat!(U8),
        NumericKind::U16 => cat!(U16),
        NumericKind::U32 => cat!(U32),
        NumericKind::U64 => cat!(U64),
        NumericKind::F32 => cat!(F32),
        NumericKind::F64 => cat!(F64),
    }
}

#[cfg(test)]
mod inventory_core_tests {
    use super::*;

    fn open(rel: &str) -> ReadOnlyHdfSession {
        ReadOnlyHdfSession::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(rel),
        )
        .unwrap()
    }

    #[test]
    fn core_inventory_differs_only_by_the_htab_identity_count() {
        for rel in [
            "wave2_1d/xs_fields.h5",
            "w3c/htab_station_scalar.h5",
            "w3c/htab_station_vector.h5",
            "w3c/htab_station_rank3.h5",
        ] {
            let s = open(rel);
            let core = s.xs_geometry_inventory_core(64 << 20).unwrap();
            let full = s.xs_geometry_inventory(64 << 20).unwrap();
            assert_eq!(core.geometry, full.geometry, "{rel}");
            assert_eq!(core.attributes, full.attributes, "{rel}");
            assert_eq!(core.unqualified, full.unqualified, "{rel}");
            assert_eq!(core.fields.len(), full.fields.len(), "{rel}");
            for (c, f) in core.fields.iter().zip(&full.fields) {
                let mut c = c.clone();
                if c.field == XsGeometryField::PropertyTable {
                    c.section_records = f.section_records;
                }
                assert_eq!(&c, f, "{rel} {:?}", f.field);
            }
        }
    }

    #[test]
    fn field_reads_do_not_depend_on_which_inventory_was_built_first() {
        for rel in ["wave2_1d/xs_fields.h5", "w3c/htab_station_scalar.h5"] {
            for &field in XS_GEOMETRY_FIELDS {
                let fresh = open(rel).read_xs_geometry(field, None, 64 << 20, 256 << 20);
                let s = open(rel);
                s.xs_geometry_inventory(64 << 20).unwrap();
                let after_full = s.read_xs_geometry(field, None, 64 << 20, 256 << 20);
                match (fresh, after_full) {
                    // Debug output compares stored NaN values too (NaN != NaN under PartialEq).
                    (Ok(a), Ok(b)) => {
                        assert_eq!(format!("{a:?}"), format!("{b:?}"), "{rel} {field:?}")
                    }
                    (Err(a), Err(b)) => {
                        assert_eq!(
                            (a.kind, a.to_string()),
                            (b.kind, b.to_string()),
                            "{rel} {field:?}"
                        )
                    }
                    (a, b) => panic!("{rel} {field:?}: {a:?} versus {b:?}"),
                }
            }
        }
    }
}
