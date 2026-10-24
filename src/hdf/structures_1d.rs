//! 1D structures: exact `Geometry/Structures` tables with per-structure identities and
//! profile ranges, exact pump-station and storage-area geometry groups, and the result series
//! families the qualified `hdf::structures` module does not read: unsteady `Inline
//! Structures`, `Bridges`, `Culverts` and `Pumping Stations`, and the steady `Steady
//! Profiles` structure families (lateral structures, storage areas, and inline/bridge/culvert
//! groups when present). Series conventions (rank-2 `(row, column)` datasets, `Variable_Unit`
//! / `Connections` column labels, `Units`-only dataset units, `Geometric Info` excluded,
//! `Node Pointer`) are exactly those of `hdf::structures`.
use super::mesh_geometry::{exact_compound, ExactCompound};
use super::metadata::{CompoundTable, MetadataValue, TypedAttribute};
use super::structures::{GeometryIdentity, StructureDescriptor, StructureSeries, Walk};
use super::xs1d::{STEADY, UNSTEADY};
use super::xs_geometry::{
    budget, checked_range, corrupt, exact_array, invalid, reported_text, unsupported, width,
};
use super::{Identity, NumericArray, NumericKind, ReadOnlyHdfSession, ResultMode, Slice};
use crate::capability::error_to_capability;
use crate::{Capability, Error, ErrorKind, Reported, Result, UnsupportedCode};
use std::collections::BTreeSet;
use std::ops::Range;

const STRUCTURES: &str = "Geometry/Structures";
const STRUCTURE_ATTRIBUTES: &str = "Geometry/Structures/Attributes";
const MAX_OBJECTS: usize = 4096;
const MAX_STRUCTURES: usize = 50_000;

/// Result families of 1D structures read by this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum OneDStructureKind {
    /// `Inline Structures` (identity: `Type == Inline` rows).
    InlineStructure,
    /// `Bridges` (`Type == Bridge`).
    Bridge,
    /// `Culverts` (`Type == Culvert`).
    Culvert,
    /// `Pumping Stations` (identity: `Geometry/Pump Stations/Attributes` `Name`).
    PumpStation,
    /// `Lateral Structures` (`Type == Lateral`); steady mode only here (unsteady lateral
    /// structures are the qualified `structure_inventory` family).
    LateralStructure,
    /// `Storage Areas` (`Geometry/Storage Areas/Attributes` `Name`); steady mode only here.
    StorageArea,
}
pub const ONE_D_STRUCTURE_KINDS: &[OneDStructureKind] = &[
    OneDStructureKind::InlineStructure,
    OneDStructureKind::Bridge,
    OneDStructureKind::Culvert,
    OneDStructureKind::PumpStation,
    OneDStructureKind::LateralStructure,
    OneDStructureKind::StorageArea,
];
impl OneDStructureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InlineStructure => "inline_structure",
            Self::Bridge => "bridge",
            Self::Culvert => "culvert",
            Self::PumpStation => "pump_station",
            Self::LateralStructure => "lateral_structure",
            Self::StorageArea => "storage_area",
        }
    }
    pub fn group_name(self) -> &'static str {
        match self {
            Self::InlineStructure => "Inline Structures",
            Self::Bridge => "Bridges",
            Self::Culvert => "Culverts",
            Self::PumpStation => "Pumping Stations",
            Self::LateralStructure => "Lateral Structures",
            Self::StorageArea => "Storage Areas",
        }
    }
    /// Geometry attribute table that identifies the structure groups.
    pub fn geometry_table(self) -> &'static str {
        match self {
            Self::PumpStation => "Geometry/Pump Stations/Attributes",
            Self::StorageArea => "Geometry/Storage Areas/Attributes",
            _ => STRUCTURE_ATTRIBUTES,
        }
    }
    fn type_name(self) -> Option<&'static str> {
        match self {
            Self::InlineStructure => Some("Inline"),
            Self::Bridge => Some("Bridge"),
            Self::Culvert => Some("Culvert"),
            Self::LateralStructure => Some("Lateral"),
            _ => None,
        }
    }
    fn allowed(self, mode: ResultMode) -> bool {
        !(mode == ResultMode::Unsteady
            && matches!(self, Self::LateralStructure | Self::StorageArea))
    }
    /// True only for (kind, mode) layouts qualified by real HEC-RAS files: unsteady inline
    /// structures, bridges, culverts and pumping stations; steady lateral structures and
    /// storage areas. Any other present family is Unsupported(identity_layout) until a real
    /// fixture qualifies it (no local steady plan writes inline/bridge/culvert/pump groups).
    pub fn qualified(self, mode: ResultMode) -> bool {
        match mode {
            ResultMode::Unsteady => matches!(
                self,
                Self::InlineStructure | Self::Bridge | Self::Culvert | Self::PumpStation
            ),
            ResultMode::Steady => matches!(self, Self::LateralStructure | Self::StorageArea),
        }
    }
    /// Full path of the family group for a mode.
    pub fn group_path(self, mode: ResultMode) -> String {
        let base = match mode {
            ResultMode::Steady => STEADY,
            ResultMode::Unsteady => UNSTEADY,
        };
        format!("{base}/{}", self.group_name())
    }
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OneDStructureInventory {
    pub kind: OneDStructureKind,
    pub mode: ResultMode,
    pub group: String,
    /// Stored unsteady time rows or steady profile count.
    pub rows: u64,
    pub structures: Vec<StructureDescriptor>,
    /// Family-level datasets (steady storage areas `Flow`, `Water Surface`); column order is
    /// not asserted to follow any structure ordering.
    pub family_series: Vec<StructureSeries>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OneDStructureCapability {
    pub kind: OneDStructureKind,
    pub mode: ResultMode,
    pub group: String,
    pub status: Capability,
}
/// One `Geometry/Structures/Attributes` row with exact identity text and profile ranges.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StructureRecord {
    pub id: Identity,
    pub structure_type: Reported<String>,
    pub river: Reported<String>,
    pub reach: Reported<String>,
    pub station: Reported<String>,
    pub connection: Reported<String>,
    pub node_name: Reported<String>,
    /// `Table Info` `X (Index)`/`X (Count)` pairs as validated ranges into `Profile Data`.
    pub profile_ranges: Vec<(String, Range<u64>)>,
}
/// An exact geometry group: every compound table and numeric array (recursively), with
/// typed group attributes. Datasets of other encodings are listed with their capability.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct GeometryGroupData {
    pub group: String,
    pub tables: Vec<(String, ExactCompound)>,
    pub arrays: Vec<(String, NumericArray)>,
    pub group_attributes: Vec<(String, Vec<TypedAttribute>)>,
    /// Objects not returned, each with its own capability: unproven encodings and byte-budget
    /// refusals (Unsupported with their code) and damaged storage (Corrupt). One policy for
    /// tables, arrays and group attributes (`<group>@attributes`); only host errors and the
    /// depth/object limits fail the whole group.
    pub unsupported: Vec<(String, Capability)>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StructureGeometry {
    pub attributes: ExactCompound,
    pub records: Vec<StructureRecord>,
    /// Every table/array of `Geometry/Structures` (Attributes included), exact.
    pub group: GeometryGroupData,
}
/// Qualified 1D geometry groups for the exact group reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OneDGeometryGroup {
    Structures,
    PumpStations,
    StorageAreas,
}
impl OneDGeometryGroup {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Structures => "structures",
            Self::PumpStations => "pump_stations",
            Self::StorageAreas => "storage_areas",
        }
    }
    pub fn group_path(self) -> &'static str {
        match self {
            Self::Structures => STRUCTURES,
            Self::PumpStations => "Geometry/Pump Stations",
            Self::StorageAreas => "Geometry/Storage Areas",
        }
    }
}

/// Named `Table Info` ranges of one structure.
type ProfileRanges = Vec<(String, Range<u64>)>;

fn texts(t: &CompoundTable, name: &str) -> Result<Vec<String>> {
    super::structures::text_of(t, name)
}

impl ReadOnlyHdfSession {
    /// Exact recursive dump of one qualified geometry group (depth 4, 4096 objects). Compound
    /// datasets keep raw bytes; numeric datasets keep stored kinds; every other encoding is
    /// listed as unsupported. `max_bytes` bounds each dataset/attribute. Absent: Unavailable.
    pub fn one_d_geometry_group(
        &self,
        group: OneDGeometryGroup,
        max_bytes: usize,
    ) -> Result<Reported<GeometryGroupData>> {
        let root = group.group_path();
        if !self.has_group(root)? {
            return Ok(Reported::unavailable("canonical geometry group absent"));
        }
        let mut out = GeometryGroupData {
            group: root.into(),
            tables: vec![],
            arrays: vec![],
            group_attributes: vec![],
            unsupported: vec![],
        };
        let mut pending = vec![(root.to_string(), String::new(), 0usize)];
        let mut objects = 0usize;
        while let Some((g, rel, depth)) = pending.pop() {
            let attrs = self.typed_group_attributes(&g)?;
            let over = attrs
                .iter()
                .find_map(|a| budget(a.raw_bytes.len() as u64, max_bytes).err());
            match over {
                // Over-budget group attributes are listed like any other per-object failure.
                Some(e) => out
                    .unsupported
                    .push((format!("{rel}@attributes"), error_to_capability(e)?)),
                None => out.group_attributes.push((rel.clone(), attrs)),
            }
            let m = self.group_members(&g)?;
            for d in m.datasets {
                objects += 1;
                if objects > MAX_OBJECTS {
                    return Err(unsupported(
                        UnsupportedCode::ResourceLimit,
                        "geometry group object limit",
                    ));
                }
                let r = if rel.is_empty() {
                    d.clone()
                } else {
                    format!("{rel}/{d}")
                };
                let path = format!("{g}/{d}");
                match exact_compound(self, &path, max_bytes) {
                    Ok(c) => {
                        out.tables.push((r, c));
                        continue;
                    }
                    Err(e)
                        if e.kind == ErrorKind::Unsupported
                            && e.code == Some(UnsupportedCode::Schema) => {}
                    Err(e) => {
                        out.unsupported.push((r, error_to_capability(e)?));
                        continue;
                    }
                }
                // Same policy as compounds: a per-dataset failure (unproven encoding, byte
                // budget, damaged storage) is listed with its capability; only host errors
                // fail the whole group.
                let cap = self.numeric_capability(&path)?;
                if !cap.capability.is_supported() {
                    out.unsupported.push((r, cap.capability));
                    continue;
                }
                match exact_array(self, &path, max_bytes) {
                    Ok(a) => out.arrays.push((r, a)),
                    Err(e) => out.unsupported.push((r, error_to_capability(e)?)),
                }
            }
            for s in m.groups {
                if depth >= 4 {
                    return Err(unsupported(
                        UnsupportedCode::ResourceLimit,
                        "geometry group nesting limit",
                    ));
                }
                let r = if rel.is_empty() {
                    s.clone()
                } else {
                    format!("{rel}/{s}")
                };
                pending.push((format!("{g}/{s}"), r, depth + 1));
            }
        }
        out.tables.sort_by(|a, b| a.0.cmp(&b.0));
        out.arrays.sort_by(|a, b| a.0.cmp(&b.0));
        out.group_attributes.sort_by(|a, b| a.0.cmp(&b.0));
        out.unsupported.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Reported::supported(out))
    }
    /// `Geometry/Structures` with one typed record per Attributes row (exact Type, River,
    /// Reach, RS, Connection, Node Name) and validated `Table Info` profile ranges into
    /// `Profile Data`, plus the exact group dump. Absent group or Attributes: Unavailable.
    pub fn structure_geometry(&self, max_bytes: usize) -> Result<Reported<StructureGeometry>> {
        if !self.has_group(STRUCTURES)? {
            return Ok(Reported::unavailable("structure geometry absent"));
        }
        if !self.has_dataset(STRUCTURE_ATTRIBUTES)? {
            return Ok(Reported::unavailable("structure attribute table absent"));
        }
        let attributes = exact_compound(self, STRUCTURE_ATTRIBUTES, max_bytes)?;
        let t = &attributes.table;
        if t.shape.len() != 1 {
            return Err(corrupt("structure attributes must be rank one"));
        }
        let n = t.shape[0];
        if n as usize > MAX_STRUCTURES {
            return Err(unsupported(
                UnsupportedCode::ResourceLimit,
                "structure count limit",
            ));
        }
        let table_info = format!("{STRUCTURES}/Table Info");
        let profile = format!("{STRUCTURES}/Profile Data");
        let ranges: Option<Vec<ProfileRanges>> = if self.has_dataset(&table_info)? {
            let ti = exact_compound(self, &table_info, max_bytes)?;
            if ti.table.shape != [n] {
                return Err(corrupt("Table Info and Attributes row counts differ"));
            }
            let bound = if self.has_dataset(&profile)? {
                self.numeric_capability(&profile)?
                    .shape
                    .first()
                    .copied()
                    .unwrap_or(0)
            } else {
                0
            };
            let mut per = vec![Vec::new(); n as usize];
            for c in &ti.table.columns {
                let Some(base) = c.name.strip_suffix(" (Index)") else {
                    continue;
                };
                let Some(count) = ti
                    .table
                    .columns
                    .iter()
                    .find(|k| k.name == format!("{base} (Count)"))
                else {
                    return Err(corrupt(format!("Table Info {base} has no Count column")));
                };
                let (MetadataValue::Signed(iv), MetadataValue::Signed(cv)) =
                    (&c.value, &count.value)
                else {
                    return Err(unsupported(
                        UnsupportedCode::Encoding,
                        "Table Info columns are not proven integers",
                    ));
                };
                for row in 0..n as usize {
                    per[row].push((base.to_string(), checked_range(iv[row], cv[row], bound)?));
                }
            }
            Some(per)
        } else {
            None
        };
        let mut records = Vec::with_capacity(n as usize);
        for row in 0..n as usize {
            records.push(StructureRecord {
                id: Identity::new(self.path(), STRUCTURE_ATTRIBUTES, row as u64),
                structure_type: reported_text(t, "Type", row)?,
                river: reported_text(t, "River", row)?,
                reach: reported_text(t, "Reach", row)?,
                station: reported_text(t, "RS", row)?,
                connection: reported_text(t, "Connection", row)?,
                node_name: reported_text(t, "Node Name", row)?,
                profile_ranges: ranges.as_ref().map(|r| r[row].clone()).unwrap_or_default(),
            });
        }
        let group = self
            .one_d_geometry_group(OneDGeometryGroup::Structures, max_bytes)?
            .into_value()
            .ok_or_else(|| corrupt("structure group vanished"))?;
        Ok(Reported::supported(StructureGeometry {
            attributes,
            records,
            group,
        }))
    }
    fn one_d_rows(&self, mode: ResultMode, max_bytes: usize) -> Result<u64> {
        match mode {
            ResultMode::Unsteady => self.time_rows_for_structures(max_bytes),
            ResultMode::Steady => Ok(self.steady_profile_names(max_bytes)?.len() as u64),
        }
    }
    fn one_d_identities(
        &self,
        kind: OneDStructureKind,
        names: &[String],
        family: &str,
        max_bytes: usize,
    ) -> Result<Vec<GeometryIdentity>> {
        let table_path = kind.geometry_table();
        match self.object_kind(table_path)? {
            None => {
                return Err(unsupported(
                    UnsupportedCode::IdentityLayout,
                    format!("geometry attribute table {table_path} absent"),
                ))
            }
            Some(true) => return Err(corrupt(format!("expected dataset at {table_path}"))),
            Some(false) => {}
        }
        let t = self.simple_compound(table_path, max_bytes)?;
        if t.shape.len() != 1 {
            return Err(corrupt(format!("{table_path} must be rank one")));
        }
        let rows = t.shape[0] as usize;
        let empty = vec![String::new(); rows];
        let mut keys: Vec<(usize, String)> = Vec::new();
        let (river, reach, station, connection, name);
        if let Some(ty) = kind.type_name() {
            let types = texts(&t, "Type")?;
            river = texts(&t, "River")?;
            reach = texts(&t, "Reach")?;
            station = texts(&t, "RS")?;
            // An absent Connection column is canonical (empty); a present but unproven or
            // malformed one is surfaced, never replaced by empty text.
            connection = if t.columns.iter().any(|c| c.name == "Connection") {
                texts(&t, "Connection")?
            } else {
                empty.clone()
            };
            name = empty.clone();
            for r in 0..rows {
                if types[r].trim() == ty {
                    keys.push((
                        r,
                        format!(
                            "{} {} {}",
                            river[r].trim(),
                            reach[r].trim(),
                            station[r].trim()
                        ),
                    ));
                }
            }
        } else {
            name = texts(&t, "Name")?;
            river = empty.clone();
            reach = empty.clone();
            station = empty.clone();
            connection = empty.clone();
            for (r, n) in name.iter().enumerate() {
                keys.push((r, n.trim().to_string()));
            }
        }
        let mut seen = BTreeSet::new();
        for (_, k) in &keys {
            if !seen.insert(k.clone()) {
                return Err(unsupported(
                    UnsupportedCode::IdentityLayout,
                    format!("{table_path}: identity key {k:?} is not unique"),
                ));
            }
        }
        names
            .iter()
            .map(|g| {
                let found: Vec<usize> = keys
                    .iter()
                    .filter(|(_, k)| k == g.trim())
                    .map(|(r, _)| *r)
                    .collect();
                match found.as_slice() {
                    [r] => Ok(GeometryIdentity {
                        id: Identity::new(self.path(), table_path, *r as u64),
                        river: river[*r].clone(),
                        reach: reach[*r].clone(),
                        station: station[*r].clone(),
                        connection: connection[*r].clone(),
                        upstream: String::new(),
                        downstream: String::new(),
                        name: name[*r].clone(),
                    }),
                    _ => Err(unsupported(
                        UnsupportedCode::IdentityLayout,
                        format!("{family}/{g}: no unique geometry identity"),
                    )),
                }
            })
            .collect()
    }
    /// Structures of one 1D family with geometry identities (group name equals the
    /// space-joined trimmed River Reach RS of a matching `Type` row, or the exact `Name`),
    /// series (rank-2 rows checked against the time axis or the steady profile count) and
    /// static datasets. Absent family: NotFound. Unsteady lateral structures and storage areas
    /// are InvalidRequest here (use `structure_inventory`).
    pub fn one_d_structure_inventory(
        &self,
        kind: OneDStructureKind,
        mode: ResultMode,
        max_bytes: usize,
    ) -> Result<OneDStructureInventory> {
        if !kind.allowed(mode) {
            return Err(invalid(
                "unsteady lateral structures and storage areas use structure_inventory",
            ));
        }
        self.memo(
            format!("one_d_structure_inventory:{:?}", (kind, mode, max_bytes)),
            || {
                let family = kind.group_path(mode);
                match self.object_kind(&family)? {
                    None => {
                        return Err(Error::new(ErrorKind::NotFound, format!("{family} absent")))
                    }
                    Some(false) => return Err(corrupt(format!("expected group at {family}"))),
                    Some(true) => {}
                }
                if !kind.qualified(mode) {
                    return Err(unsupported(
                        UnsupportedCode::IdentityLayout,
                        format!("unqualified layout: no real fixture qualifies {family}"),
                    ));
                }
                let rows = self.one_d_rows(mode, max_bytes)?;
                let members = self.group_members(&family)?;
                let mut names = members.groups.clone();
                names.sort();
                if kind != OneDStructureKind::StorageArea {
                    if let Some(d) = members.datasets.first() {
                        return Err(corrupt(format!(
                            "{family}/{d}: dataset where a structure group is required"
                        )));
                    }
                }
                if names.is_empty() {
                    return Err(unsupported(
                        UnsupportedCode::Schema,
                        format!("{family} contains no structure groups"),
                    ));
                }
                if names.len() > MAX_STRUCTURES {
                    return Err(unsupported(
                        UnsupportedCode::ResourceLimit,
                        "structure count limit",
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
                let identities = self.one_d_identities(kind, &names, &family, max_bytes)?;
                let mut family_series = Vec::new();
                let mut ds = members.datasets.clone();
                ds.sort();
                for d in ds {
                    let s = self.series_for(&format!("{family}/{d}"), &d, rows, true, max_bytes)?;
                    if s.shape[1] != names.len() as u64 {
                        return Err(corrupt(format!(
                            "{}: {} columns but {} groups",
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
                            time_rows: rows,
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
                Ok(OneDStructureInventory {
                    kind,
                    mode,
                    group: family,
                    rows,
                    structures,
                    family_series,
                })
            },
        )
    }
    /// Status of every (kind, mode) family: Supported after the full inventory validates;
    /// absent Unavailable; failures isolated (host errors propagate).
    pub fn one_d_structure_capabilities(
        &self,
        max_bytes: usize,
    ) -> Result<Vec<OneDStructureCapability>> {
        let mut out = Vec::new();
        for mode in [ResultMode::Unsteady, ResultMode::Steady] {
            for &kind in ONE_D_STRUCTURE_KINDS {
                if !kind.allowed(mode) {
                    continue;
                }
                let status = match self.object_kind(&kind.group_path(mode)) {
                    Ok(None) => Capability::unavailable("family absent"),
                    Ok(Some(false)) => Capability::corrupt("expected a family group"),
                    Ok(Some(true)) => match self.one_d_structure_inventory(kind, mode, max_bytes) {
                        Ok(_) => Capability::Supported,
                        Err(e) => error_to_capability(e)?,
                    },
                    Err(e) => error_to_capability(e)?,
                };
                out.push(OneDStructureCapability {
                    kind,
                    mode,
                    group: kind.group_path(mode),
                    status,
                });
            }
        }
        Ok(out)
    }
    /// Exact selected rows/columns of one 1D structure series. `structure` is the exact group
    /// name (None only for family-level storage-area datasets); `relative` is
    /// `StructureSeries::relative`. `metadata_max_bytes` is the inventory budget; `max_bytes`
    /// bounds returned bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn read_one_d_structure_series(
        &self,
        kind: OneDStructureKind,
        mode: ResultMode,
        structure: Option<&str>,
        relative: &str,
        rows: Slice,
        columns: Option<Slice>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<NumericArray> {
        let inv = self.one_d_structure_inventory(kind, mode, metadata_max_bytes)?;
        let series = match structure {
            Some(n) => inv
                .structures
                .iter()
                .find(|s| s.name == n)
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "structure absent"))?
                .series
                .iter()
                .find(|s| s.relative == relative),
            None => inv.family_series.iter().find(|s| s.relative == relative),
        }
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "structure series absent"))?;
        match &series.capability {
            Capability::Supported => {}
            Capability::Unsupported { code, reason } => {
                return Err(unsupported(*code, reason.clone()))
            }
            Capability::Corrupt { reason } => return Err(corrupt(reason.clone())),
            c => {
                return Err(corrupt(
                    c.reason()
                        .unwrap_or("structure series not readable")
                        .to_string(),
                ))
            }
        }
        let kindn = series
            .kind
            .ok_or_else(|| unsupported(UnsupportedCode::Encoding, "unproven kind"))?;
        if !matches!(kindn, NumericKind::F32 | NumericKind::F64) {
            return Err(unsupported(
                UnsupportedCode::Encoding,
                "structure series must be f32/f64",
            ));
        }
        let cols = columns.unwrap_or(Slice::new(0, series.shape[1], 1));
        let mut n = 1u64;
        for (s, dim) in [(rows, series.shape[0]), (cols, series.shape[1])] {
            if s.step == 0 || s.start > s.end || s.end > dim {
                return Err(invalid("invalid structure series selection"));
            }
            n *= (s.end - s.start).div_ceil(s.step);
        }
        budget(n * width(kindn), max_bytes)?;
        if n == 0 {
            return Ok(NumericArray {
                shape: vec![
                    (rows.end - rows.start).div_ceil(rows.step),
                    (cols.end - cols.start).div_ceil(cols.step),
                ],
                values: super::xs_geometry::concat(Vec::new(), kindn),
            });
        }
        self.read_numeric(&series.path, Some(&[rows, cols]))
    }
    /// Pump stations geometry: exact Attributes + Points (one per station) and names.
    pub fn pump_stations(
        &self,
        max_bytes: usize,
    ) -> Result<Reported<super::xs_geometry::OneDPointData>> {
        self.point_family("Geometry/Pump Stations", max_bytes)
    }
}
