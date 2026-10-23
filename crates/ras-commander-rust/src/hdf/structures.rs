//! Read-only unsteady structure result families (SA/2D area connections, lateral structures
//! and storage areas). Layouts are those observed in stored HEC-RAS 6.x result files; nothing
//! here interprets hydraulics: variable names, column labels and units are reported exactly as
//! stored, and identity links to the geometry attribute tables are evidence, not repairs.
use super::metadata::{typed_attribute, CompoundTable, MetadataValue};
use super::units::{evidence, DatasetUnits, UnitsSource};
use super::xs1d::UNSTEADY;
use super::{NumericArray, NumericKind, ReadOnlyHdfSession, Slice};
use crate::capability::{absent_ok, error_to_capability};
use crate::{Capability, Error, ErrorKind, Result, UnsupportedCode};
use std::collections::{BTreeMap, BTreeSet};

/// Structure families whose stored layout is qualified by real files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum StructureFamilyKind {
    /// `SA 2D Area Conn`
    Sa2dAreaConnection,
    /// `Lateral Structures`
    LateralStructure,
    /// `Storage Areas`
    StorageArea,
}
impl StructureFamilyKind {
    /// Every qualified family, in reporting order.
    pub const ALL: [StructureFamilyKind; 3] = [
        Self::Sa2dAreaConnection,
        Self::LateralStructure,
        Self::StorageArea,
    ];
    /// Stable snake_case spelling (never the Debug form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sa2dAreaConnection => "sa_2d_area_connection",
            Self::LateralStructure => "lateral_structure",
            Self::StorageArea => "storage_area",
        }
    }
    /// Group name below the unsteady time series group.
    pub fn group_name(self) -> &'static str {
        match self {
            Self::Sa2dAreaConnection => "SA 2D Area Conn",
            Self::LateralStructure => "Lateral Structures",
            Self::StorageArea => "Storage Areas",
        }
    }
    /// Geometry attribute table that carries the identity columns.
    pub fn geometry_table(self) -> &'static str {
        match self {
            Self::StorageArea => "Geometry/Storage Areas/Attributes",
            _ => "Geometry/Structures/Attributes",
        }
    }
    /// Full path of the family group.
    pub fn group_path(self) -> String {
        format!("{UNSTEADY}/{}", self.group_name())
    }
}
/// Family group names whose layout is NOT qualified on real data; presence is Unsupported.
pub const UNQUALIFIED_STRUCTURE_GROUPS: [&str; 5] = [
    "Structures",
    "Inline Structures",
    "Bridges",
    "Culverts",
    "Bridges/Culverts",
];
const MAX_STRUCTURES: usize = 50_000;
const MAX_SERIES_PER_STRUCTURE: usize = 512;
const MAX_DEPTH: usize = 4;
const GEOMETRIC_INFO: &str = "Geometric Info";

/// Where per-column labels were read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColumnLabelSource {
    /// `Variable_Unit`: (columns, 2) fixed text of [name, unit].
    VariableUnit,
    /// `Connections`: one fixed text name per column, no units.
    Connections,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColumnLabel {
    pub name: String,
    pub unit: Option<String>,
}
/// Declared labels with the attribute they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColumnLabelSet {
    pub source: ColumnLabelSource,
    pub labels: Vec<ColumnLabel>,
}
/// Per-column labels of one series, exactly as stored, with a single status: Supported
/// (declared labels plus the stored raw bytes), Unavailable (no label attribute),
/// Unsupported or Corrupt (typed reason plus the stored raw bytes).
pub type ColumnLabels = crate::Reported<ColumnLabelSet>;
/// One `(time row, column)` dataset below a structure (or at family level for storage areas).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StructureSeries {
    /// Full HDF path.
    pub path: String,
    /// Path relative to the structure group (or family group for family-level series).
    pub relative: String,
    pub shape: Vec<u64>,
    /// Some only for proven numeric storage.
    pub kind: Option<NumericKind>,
    pub capability: Capability,
    pub columns: ColumnLabels,
    /// Dataset-level units: the `Units` attribute for datasets inside a structure; the shared
    /// quantity-label evidence for family-level datasets. Per-column units are in `columns`.
    pub units: DatasetUnits,
}
/// `Node Pointer` attribute of a structure group.
pub type NodePointer = crate::Reported<i64>;
/// Identity row of the geometry attribute table; fields not applicable are empty strings.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GeometryIdentity {
    /// Source file, geometry attribute table and the row of this structure.
    pub id: super::Identity,
    pub river: String,
    pub reach: String,
    pub station: String,
    pub connection: String,
    pub upstream: String,
    pub downstream: String,
    /// Storage-area `Name`; empty elsewhere.
    pub name: String,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StructureDescriptor {
    /// Exact group name below the family group.
    pub name: String,
    pub group: String,
    pub node_pointer: NodePointer,
    pub identity: GeometryIdentity,
    pub series: Vec<StructureSeries>,
    /// Rank-1 (non time series) datasets outside `Geometric Info`, relative paths.
    pub static_datasets: Vec<String>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StructureInventory {
    pub kind: StructureFamilyKind,
    pub group: String,
    /// Row count of the validated unsteady time axis.
    pub time_rows: u64,
    pub structures: Vec<StructureDescriptor>,
    /// Datasets directly in the family group (storage areas: `Flow`, `Water Surface`).
    /// Column order is not asserted to follow any structure ordering.
    pub family_series: Vec<StructureSeries>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StructureCapability {
    pub kind: StructureFamilyKind,
    pub status: Capability,
    /// Group paths found for this family.
    pub datasets: Vec<String>,
    /// Series-level findings that do not change `status`: Supported certifies the identity,
    /// time-row and dtype layout, NOT that every column-label table is valid. Each entry names
    /// a series whose `ColumnLabels` is Corrupt or Unsupported (inspect the inventory).
    pub diagnostics: Vec<String>,
}

fn corrupt(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt, m)
}
fn unsup(code: UnsupportedCode, m: impl Into<String>) -> Error {
    Error::unsupported(code, m)
}
type FResult<T> = Result<T>;
/// Per-inventory constants threaded through the structure walk.
#[derive(Clone, Copy)]
pub(super) struct Walk {
    pub(super) time_rows: u64,
    pub(super) max_bytes: usize,
}

/// Series whose column-label table is not valid; reported beside a Supported status.
fn label_diagnostics(inv: &StructureInventory) -> Vec<String> {
    let mut out = Vec::new();
    let all = inv
        .structures
        .iter()
        .flat_map(|s| s.series.iter())
        .chain(inv.family_series.iter());
    for s in all {
        let cap = s.columns.capability();
        let what = match cap {
            Capability::Corrupt { .. } => "corrupt",
            Capability::Unsupported { .. } => "unsupported",
            _ => continue,
        };
        let reason = cap.reason().unwrap_or("");
        out.push(format!("{}: column labels {what}: {reason}", s.path));
    }
    out
}

pub(super) fn text_of(t: &CompoundTable, name: &str) -> FResult<Vec<String>> {
    let c = t.columns.iter().find(|c| c.name == name).ok_or_else(|| {
        unsup(
            UnsupportedCode::IdentityLayout,
            format!("geometry attribute field {name} missing"),
        )
    })?;
    match &c.value {
        MetadataValue::Strings(v) if t.shape.len() == 1 && v.len() as u64 == t.shape[0] => {
            Ok(v.clone())
        }
        _ => Err(unsup(
            UnsupportedCode::IdentityLayout,
            format!("geometry attribute field {name} is not proven fixed text"),
        )),
    }
}

impl ReadOnlyHdfSession {
    pub(super) fn time_rows_for_structures(&self, max_bytes: usize) -> FResult<u64> {
        match absent_ok(self.unsteady_time_axis(max_bytes))? {
            Some(axis) => Ok(axis.rows),
            None => Err(corrupt(
                "structure results present without a readable unsteady time axis",
            )),
        }
    }
    pub(super) fn series_for(
        &self,
        path: &str,
        relative: &str,
        time_rows: u64,
        family_level: bool,
        max_bytes: usize,
    ) -> FResult<StructureSeries> {
        let cap = self.numeric_capability(path)?;
        if cap.shape.len() != 2 {
            return Err(unsup(
                UnsupportedCode::Schema,
                format!(
                    "{path}: structure series must be rank 2 (found rank {})",
                    cap.shape.len()
                ),
            ));
        }
        if cap.shape[0] != time_rows {
            return Err(corrupt(format!(
                "{path}: {} rows but the time axis has {time_rows}",
                cap.shape[0]
            )));
        }
        let ds = self
            .file
            .dataset(path)
            .map_err(super::encoding::map_error)?;
        let attrs = ds.raw_attributes();
        let units = if family_level {
            self.unit_evidence_bounded(path, max_bytes, true)?
        } else {
            let units = evidence(attrs, "Units", max_bytes)?;
            DatasetUnits {
                source: if units.is_unavailable() {
                    UnitsSource::None
                } else {
                    UnitsSource::UnitsAttribute
                },
                units,
                quantity_name: None,
            }
        };
        let columns = column_labels(attrs, cap.shape[1])?;
        Ok(StructureSeries {
            path: path.into(),
            relative: relative.into(),
            shape: cap.shape.clone(),
            kind: cap.kind,
            capability: cap.capability.clone(),
            columns,
            units,
        })
    }
    pub(super) fn walk_structure(
        &self,
        group: &str,
        rel: &str,
        depth: usize,
        ctx: Walk,
        series: &mut Vec<StructureSeries>,
        statics: &mut Vec<String>,
    ) -> FResult<()> {
        if depth > MAX_DEPTH {
            return Err(unsup(
                UnsupportedCode::ResourceLimit,
                format!("{group}: structure nesting exceeds {MAX_DEPTH}"),
            ));
        }
        let members = self.group_members(group)?;
        let mut datasets = members.datasets;
        datasets.sort();
        let mut groups = members.groups;
        groups.sort();
        for name in datasets {
            let path = format!("{group}/{name}");
            let relative = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let ds = self
                .file
                .dataset(&path)
                .map_err(super::encoding::map_error)?;
            if ds.shape().len() == 1 {
                statics.push(relative);
                continue;
            }
            if series.len() >= MAX_SERIES_PER_STRUCTURE {
                return Err(unsup(
                    UnsupportedCode::ResourceLimit,
                    format!("{group}: more than {MAX_SERIES_PER_STRUCTURE} series"),
                ));
            }
            series.push(self.series_for(&path, &relative, ctx.time_rows, false, ctx.max_bytes)?);
        }
        for name in groups {
            if depth == 0 && name == GEOMETRIC_INFO {
                continue;
            }
            let relative = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            self.walk_structure(
                &format!("{group}/{name}"),
                &relative,
                depth + 1,
                ctx,
                series,
                statics,
            )?;
        }
        Ok(())
    }
    pub(super) fn node_pointer(&self, group: &str) -> FResult<NodePointer> {
        let g = self.file.group(group).map_err(super::encoding::map_error)?;
        let attrs = g.raw_attributes().map_err(super::encoding::map_error)?;
        let mut found = attrs.into_iter().filter(|a| a.name == "Node Pointer");
        let Some(first) = found.next() else {
            return Ok(NodePointer::unavailable("no Node Pointer attribute"));
        };
        if found.next().is_some() {
            return Err(corrupt(format!(
                "{group}: duplicate Node Pointer attribute"
            )));
        }
        let t = typed_attribute(first)?;
        let raw = t.raw_bytes;
        Ok(match t.value {
            MetadataValue::Signed(v) if v.len() == 1 => {
                NodePointer::supported(v[0]).with_raw_bytes(raw)
            }
            MetadataValue::Unsigned(v) if v.len() == 1 => NodePointer::supported(
                i64::try_from(v[0]).map_err(|_| corrupt("Node Pointer exceeds i64"))?,
            )
            .with_raw_bytes(raw),
            MetadataValue::Corrupt { reason } => return Err(corrupt(reason)),
            MetadataValue::Unsupported { code, reason } => {
                NodePointer::unsupported(code, reason).with_raw_bytes(raw)
            }
            _ => NodePointer::unsupported(
                UnsupportedCode::Encoding,
                "Node Pointer is not one proven integer",
            )
            .with_raw_bytes(raw),
        })
    }

    fn structure_inventory_inner(
        &self,
        kind: StructureFamilyKind,
        max_bytes: usize,
    ) -> FResult<StructureInventory> {
        self.memo(
            format!("structure_inventory_inner:{:?}", (kind, max_bytes,)),
            || {
                let family = kind.group_path();
                match self.object_kind(&family)? {
                    None => {
                        return Err(Error::new(ErrorKind::NotFound, format!("{family} absent")))
                    }
                    Some(false) => return Err(corrupt(format!("expected group at {family}"))),
                    Some(true) => {}
                }
                let time_rows = self.time_rows_for_structures(max_bytes)?;
                let members = self.group_members(&family)?;
                let mut names = members.groups.clone();
                names.sort();
                if kind != StructureFamilyKind::StorageArea {
                    if let Some(d) = members.datasets.first() {
                        return Err(corrupt(format!(
                            "{family}/{d}: dataset where a structure group is required"
                        )));
                    }
                }
                if names.is_empty() {
                    return Err(unsup(
                        UnsupportedCode::Schema,
                        format!("{family} contains no structure groups"),
                    ));
                }
                if names.len() > MAX_STRUCTURES {
                    return Err(unsup(
                        UnsupportedCode::ResourceLimit,
                        format!("{family} has more than {MAX_STRUCTURES} structures"),
                    ));
                }
                let mut seen = BTreeSet::new();
                for n in &names {
                    if !seen.insert(n.trim()) {
                        return Err(corrupt(format!(
                            "{family}: duplicate trimmed name {:?}",
                            n.trim()
                        )));
                    }
                }
                let table_path = kind.geometry_table();
                match self.object_kind(table_path)? {
                    None => {
                        return Err(unsup(
                            UnsupportedCode::IdentityLayout,
                            format!("geometry attribute table {table_path} absent"),
                        ))
                    }
                    Some(true) => return Err(corrupt(format!("expected dataset at {table_path}"))),
                    Some(false) => {}
                }
                let table = self.simple_compound(table_path, max_bytes)?;
                if table.shape.len() != 1 {
                    return Err(corrupt(format!("{table_path} must be rank one")));
                }
                let identities = geometry_identities(self.path(), kind, &table, &names, &family)?;
                let mut family_series = Vec::new();
                let mut family_ds = members.datasets.clone();
                family_ds.sort();
                for d in family_ds {
                    let s =
                        self.series_for(&format!("{family}/{d}"), &d, time_rows, true, max_bytes)?;
                    if s.shape[1] != names.len() as u64 {
                        return Err(corrupt(format!(
                            "{}: {} columns but {} storage area groups",
                            s.path,
                            s.shape[1],
                            names.len()
                        )));
                    }
                    family_series.push(s);
                }
                let mut structures = Vec::with_capacity(names.len());
                let mut pointers = BTreeSet::new();
                for (name, identity) in names.iter().zip(identities) {
                    let group = format!("{family}/{name}");
                    let node_pointer = self.node_pointer(&group)?;
                    if let Some(&p) = node_pointer.value() {
                        if !pointers.insert(p) {
                            return Err(corrupt(format!("{family}: duplicate Node Pointer {p}")));
                        }
                    }
                    let mut series = Vec::new();
                    let mut static_datasets = Vec::new();
                    self.walk_structure(
                        &group,
                        "",
                        0,
                        Walk {
                            time_rows,
                            max_bytes,
                        },
                        &mut series,
                        &mut static_datasets,
                    )?;
                    structures.push(StructureDescriptor {
                        name: name.clone(),
                        group,
                        node_pointer,
                        identity,
                        series,
                        static_datasets,
                    });
                }
                Ok(StructureInventory {
                    kind,
                    group: family,
                    time_rows,
                    structures,
                    family_series,
                })
            },
        )
    }
    /// Enumerates the structures of one family with identities from the geometry attribute
    /// table, variable series (shape/dtype/labels/units) and the validated time-row count.
    /// `max_bytes` bounds the geometry attribute table. Any identity/count/row disagreement
    /// is Corrupt; unproven layouts are Unsupported; an absent family is NotFound.
    pub fn structure_inventory(
        &self,
        kind: StructureFamilyKind,
        max_bytes: usize,
    ) -> Result<StructureInventory> {
        self.structure_inventory_inner(kind, max_bytes)
    }
    /// Exact F32/F64 selected values of one structure series. `structure` is the exact group
    /// name (None only for storage-area family-level datasets); `relative` is
    /// `StructureSeries::relative`. `max_bytes` bounds the returned bytes; no value is
    /// converted or transposed.
    pub fn read_structure_series(
        &self,
        kind: StructureFamilyKind,
        structure: Option<&str>,
        relative: &str,
        rows: Slice,
        columns: Slice,
        max_bytes: usize,
    ) -> Result<NumericArray> {
        let invalid = |m: &str| Error::new(ErrorKind::InvalidRequest, m.to_string());
        if relative.is_empty()
            || relative
                .split('/')
                .any(|c| c.is_empty() || c == "." || c == "..")
            || relative.split('/').next() == Some(GEOMETRIC_INFO)
        {
            return Err(invalid("invalid structure series path"));
        }
        let family = kind.group_path();
        let path = match structure {
            Some(name) => {
                let members = self.group_members(&family)?;
                if !members.groups.iter().any(|g| g == name) {
                    return Err(Error::new(
                        ErrorKind::NotFound,
                        format!("structure {name:?} not in {family}"),
                    ));
                }
                format!("{family}/{name}/{relative}")
            }
            None => {
                if relative.contains('/') || kind != StructureFamilyKind::StorageArea {
                    return Err(invalid(
                        "family-level series are storage-area datasets only",
                    ));
                }
                format!("{family}/{relative}")
            }
        };
        match self.object_kind(&path)? {
            None => return Err(Error::new(ErrorKind::NotFound, format!("{path} absent"))),
            Some(true) => {
                return Err(Error::new(
                    ErrorKind::InvalidRequest,
                    format!("{path} is a group, not a series dataset"),
                ))
            }
            Some(false) => {}
        }
        let cap = self.numeric_capability(&path)?;
        if cap.shape.len() != 2 {
            return Err(Error::unsupported(
                UnsupportedCode::Schema,
                "structure series must be rank 2",
            ));
        }
        let time = self
            .file
            .dataset(&format!("{UNSTEADY}/Time"))
            .map_err(super::encoding::map_error)?;
        if time.shape() != [cap.shape[0]] {
            return Err(Error::new(
                ErrorKind::Corrupt,
                format!("{path}: row count disagrees with the time axis"),
            ));
        }
        let width = match cap.kind {
            Some(NumericKind::F32) => 4u64,
            Some(NumericKind::F64) => 8,
            _ => {
                return Err(Error::unsupported(
                    UnsupportedCode::Encoding,
                    "structure series dtype must be proven f32/f64",
                ))
            }
        };
        let mut elements = 1u64;
        for (i, s) in [rows, columns].iter().enumerate() {
            if s.step == 0 || s.start > s.end || s.end > cap.shape[i] {
                return Err(invalid("invalid structure series selection"));
            }
            elements = elements
                .checked_mul((s.end - s.start).div_ceil(s.step))
                .ok_or_else(|| invalid("structure selection size overflow"))?;
        }
        if elements
            .checked_mul(width)
            .is_none_or(|n| n > max_bytes as u64)
        {
            return Err(invalid("structure selected result exceeds max_bytes"));
        }
        self.read_numeric(&path, Some(&[rows, columns]))
    }
    /// Per-family capability for `plan_capabilities`: Supported only after the full inventory
    /// validates. Absent family: Unavailable. Failures are isolated to the family.
    pub(super) fn structure_family_capability(
        &self,
        kind: StructureFamilyKind,
        max_bytes: usize,
    ) -> Result<StructureCapability> {
        let path = kind.group_path();
        let mut diagnostics = Vec::new();
        let status = match self.object_kind(&path) {
            Err(e) => error_to_capability(e)?,
            Ok(None) => {
                return Ok(StructureCapability {
                    kind,
                    status: Capability::unavailable(format!("{} absent", kind.group_name())),
                    datasets: Vec::new(),
                    diagnostics: Vec::new(),
                })
            }
            Ok(Some(_)) => match self.structure_inventory_inner(kind, max_bytes) {
                Ok(inv) => {
                    diagnostics = label_diagnostics(&inv);
                    Capability::Supported
                }
                Err(e) => error_to_capability(e)?,
            },
        };
        Ok(StructureCapability {
            kind,
            status,
            datasets: vec![path],
            diagnostics,
        })
    }
}

fn column_labels(
    attrs: &[hdf5_reader::messages::attribute::AttributeMessage],
    columns: u64,
) -> FResult<ColumnLabels> {
    let mut found = attrs
        .iter()
        .filter(|a| a.name == "Variable_Unit" || a.name == "Connections");
    let Some(a) = found.next() else {
        return Ok(ColumnLabels::unavailable("no column-label attribute"));
    };
    let extra: Vec<&str> = found.map(|e| e.name.as_str()).collect();
    let source = if a.name == "Variable_Unit" {
        ColumnLabelSource::VariableUnit
    } else {
        ColumnLabelSource::Connections
    };
    let t = typed_attribute(a.clone())?;
    let raw = t.raw_bytes.clone();
    let bad = |m: String| ColumnLabels::corrupt(m).with_raw_bytes(raw.clone());
    if !extra.is_empty() {
        return Ok(bad(format!(
            "multiple column-label attributes present: {} plus {}",
            a.name,
            extra.join(", ")
        )));
    }
    let declared = |labels: Vec<ColumnLabel>| {
        ColumnLabels::supported(ColumnLabelSet { source, labels }).with_raw_bytes(raw.clone())
    };
    Ok(match t.value.clone() {
        MetadataValue::Strings(v) => match source {
            ColumnLabelSource::VariableUnit if t.shape == vec![columns, 2] => declared(
                v.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| ColumnLabel {
                        name: p[0].clone(),
                        unit: Some(p[1].clone()),
                    })
                    .collect(),
            ),
            ColumnLabelSource::Connections if t.shape == vec![columns] => declared(
                v.into_iter()
                    .map(|name| ColumnLabel { name, unit: None })
                    .collect(),
            ),
            _ => bad(format!(
                "{} shape {:?} does not match {columns} columns",
                a.name, t.shape
            )),
        },
        MetadataValue::Corrupt { reason } => bad(reason),
        MetadataValue::Unsupported { code, reason } => {
            ColumnLabels::unsupported(code, reason).with_raw_bytes(raw.clone())
        }
        _ => ColumnLabels::unsupported(
            UnsupportedCode::Encoding,
            format!("{} is not proven fixed text", a.name),
        )
        .with_raw_bytes(raw.clone()),
    })
}

fn geometry_identities(
    source: &std::path::Path,
    kind: StructureFamilyKind,
    t: &CompoundTable,
    groups: &[String],
    family: &str,
) -> FResult<Vec<GeometryIdentity>> {
    let blank = |row: u64| GeometryIdentity {
        id: super::Identity::new(source, kind.geometry_table(), row),
        river: String::new(),
        reach: String::new(),
        station: String::new(),
        connection: String::new(),
        upstream: String::new(),
        downstream: String::new(),
        name: String::new(),
    };
    let rows_of = |kind_text: &str| -> FResult<Vec<usize>> {
        let types = text_of(t, "Type")?;
        Ok((0..types.len())
            .filter(|&i| types[i].trim() == kind_text)
            .collect())
    };
    let count_check = |label: &str, n: usize| -> FResult<()> {
        if n != groups.len() {
            return Err(corrupt(format!(
                "{} {label} rows ({n}) disagree with {} groups in {family}",
                kind.geometry_table(),
                groups.len()
            )));
        }
        Ok(())
    };
    match kind {
        StructureFamilyKind::StorageArea => {
            let names = text_of(t, "Name")?;
            count_check("Name", names.len())?;
            let mut index = BTreeMap::new();
            for (i, n) in names.iter().enumerate() {
                if index.insert(n.trim().to_string(), i).is_some() {
                    return Err(corrupt(format!(
                        "duplicate geometry storage area {:?}",
                        n.trim()
                    )));
                }
            }
            groups
                .iter()
                .map(|g| {
                    let i = *index.get(g.trim()).ok_or_else(|| {
                        unsup(
                            UnsupportedCode::IdentityLayout,
                            format!("{family}/{g} matches no geometry storage area Name"),
                        )
                    })?;
                    Ok(GeometryIdentity {
                        name: names[i].clone(),
                        ..blank(i as u64)
                    })
                })
                .collect()
        }
        StructureFamilyKind::LateralStructure => {
            let (river, reach, rs) = (
                text_of(t, "River")?,
                text_of(t, "Reach")?,
                text_of(t, "RS")?,
            );
            let rows = rows_of("Lateral")?;
            count_check("Lateral", rows.len())?;
            let mut index = BTreeMap::new();
            for &i in &rows {
                let key = format!("{} {} {}", river[i].trim(), reach[i].trim(), rs[i].trim());
                if index.insert(key.clone(), i).is_some() {
                    return Err(unsup(
                        UnsupportedCode::IdentityLayout,
                        format!(
                            "lateral geometry rows collide on the space-joined key {key:?} (identity rule unproven)"
                        ),
                    ));
                }
            }
            groups
                .iter()
                .map(|g| {
                    let i = *index.get(g.as_str()).ok_or_else(|| {
                        unsup(
                            UnsupportedCode::IdentityLayout,
                            format!(
                                "{family}/{g} matches no lateral geometry row (identity rule unproven)"
                            ),
                        )
                    })?;
                    Ok(GeometryIdentity {
                        river: river[i].clone(),
                        reach: reach[i].clone(),
                        station: rs[i].clone(),
                        ..blank(i as u64)
                    })
                })
                .collect()
        }
        StructureFamilyKind::Sa2dAreaConnection => {
            let (conn, us, ds) = (
                text_of(t, "Connection")?,
                text_of(t, "US SA/2D")?,
                text_of(t, "DS SA/2D")?,
            );
            let rows = rows_of("Connection")?;
            count_check("Connection", rows.len())?;
            let mut claimed = BTreeSet::new();
            groups
                .iter()
                .map(|g| {
                    let hits: Vec<usize> = rows
                        .iter()
                        .copied()
                        .filter(|&i| {
                            let c = conn[i].trim();
                            g == c
                                || *g == format!("{} {c}", us[i].trim())
                                || *g == format!("{} {c}", ds[i].trim())
                        })
                        .collect();
                    match hits.as_slice() {
                        [i] if claimed.insert(*i) => Ok(GeometryIdentity {
                            connection: conn[*i].clone(),
                            upstream: us[*i].clone(),
                            downstream: ds[*i].clone(),
                            ..blank(*i as u64)
                        }),
                        _ => Err(unsup(
                            UnsupportedCode::IdentityLayout,
                            format!(
                                "{family}/{g}: connection name matches {} geometry rows (identity pattern unproven)",
                                hits.len()
                            ),
                        )),
                    }
                })
                .collect()
        }
    }
}

impl ReadOnlyHdfSession {
    /// Aggregate Structures family plus per-family statuses. Unqualified or steady-root
    /// structure groups are reported Unsupported (never silently ignored).
    pub(super) fn structure_capabilities(
        &self,
        max_bytes: usize,
    ) -> Result<(super::xs1d::FamilyCapability, Vec<StructureCapability>)> {
        use super::xs1d::{FamilyCapability, ResultFamily, STEADY};
        let mut per = Vec::new();
        for kind in StructureFamilyKind::ALL {
            per.push(self.structure_family_capability(kind, max_bytes)?);
        }
        let mut datasets: Vec<String> = per.iter().flat_map(|c| c.datasets.clone()).collect();
        let mut unqualified = Vec::new();
        let mut failures = Vec::new();
        let mut probes: Vec<String> = Vec::new();
        for name in UNQUALIFIED_STRUCTURE_GROUPS {
            probes.push(format!("{UNSTEADY}/{name}"));
            probes.push(format!("{STEADY}/{name}"));
        }
        for kind in StructureFamilyKind::ALL {
            probes.push(format!("{STEADY}/{}", kind.group_name()));
        }
        for p in probes {
            match self.object_kind(&p) {
                Ok(Some(true)) => unqualified.push(p),
                Ok(Some(false)) => failures.push(format!("expected group at {p}")),
                Ok(None) => {}
                Err(e) => failures.push(
                    error_to_capability(e)?
                        .reason()
                        .unwrap_or_default()
                        .to_owned(),
                ),
            }
        }
        datasets.extend(unqualified.iter().cloned());
        let reasons = |pred: &dyn Fn(&Capability) -> bool| -> Vec<String> {
            per.iter()
                .filter(|c| pred(&c.status))
                .map(|c| {
                    format!(
                        "{}: {}",
                        c.kind.group_name(),
                        c.status.reason().unwrap_or("")
                    )
                })
                .collect()
        };
        let mut corrupt_reasons = reasons(&|s| matches!(s, Capability::Corrupt { .. }));
        corrupt_reasons.extend(failures);
        let mut unsupported_reasons = reasons(&|s| matches!(s, Capability::Unsupported { .. }));
        unsupported_reasons.extend(
            unqualified
                .iter()
                .map(|p| format!("{p}: layout not qualified on real data")),
        );
        let status = if !corrupt_reasons.is_empty() {
            Capability::corrupt(corrupt_reasons.join("; "))
        } else if !unsupported_reasons.is_empty() {
            let code = per
                .iter()
                .find_map(|c| match &c.status {
                    Capability::Unsupported { code, .. } => Some(*code),
                    _ => None,
                })
                .unwrap_or(UnsupportedCode::Schema);
            Capability::unsupported(code, unsupported_reasons.join("; "))
        } else if per.iter().any(|c| c.status.is_supported()) {
            Capability::Supported
        } else {
            Capability::unavailable("canonical family objects absent; other layouts unsupported")
        };
        Ok((
            FamilyCapability {
                family: ResultFamily::Structures,
                status,
                datasets,
            },
            per,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_budget_keeps_resource_limit_and_internal_propagates() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/structures/good.h5");
        let s = ReadOnlyHdfSession::open(path).unwrap();
        let f = s
            .structure_inventory_inner(StructureFamilyKind::LateralStructure, 1)
            .expect_err("budget of one byte must fail");
        match error_to_capability(f).unwrap() {
            Capability::Unsupported { code, .. } => {
                assert_eq!(code, UnsupportedCode::ResourceLimit)
            }
            other => panic!("{other:?}"),
        }
        let internal = Error::new(ErrorKind::Internal, "pool");
        assert_eq!(
            error_to_capability(internal).unwrap_err().kind,
            ErrorKind::Internal
        );
    }
}
