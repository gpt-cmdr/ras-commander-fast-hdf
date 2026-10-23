//! Read-only cross-section identities and row-major 1D result selections.
use super::metadata_proof as proof;
use super::{
    encoding::map_error, metadata::MetadataValue, NumericArray, NumericKind, ReadOnlyHdfSession,
    ResultMode, Slice,
};
use crate::capability::{absent_ok, error_to_capability, errors_to_capability};
use crate::{Capability, UnsupportedCode};
use crate::{Error, ErrorKind, Result};
use std::path::PathBuf;
/// Steady profile names and packed result-identity strings are never read above this size,
/// whatever `max_bytes` a caller passes (the limit before the explicit-budget change).
pub const MAX_NAME_TABLE_BYTES: usize = 8 * 1024 * 1024;
const GEOMETRY: &str = "Geometry/Cross Sections/Attributes";
pub(super) const UNSTEADY: &str =
    "Results/Unsteady/Output/Output Blocks/Base Output/Unsteady Time Series";
pub(super) const STEADY: &str = "Results/Steady/Output/Output Blocks/Base Output/Steady Profiles";
/// One optional cross-section field with a single status (see [`crate::Reported`]):
/// Supported with a value, Unavailable when the column is absent, Unsupported or Corrupt
/// with the typed reason when the stored column cannot be proven.
pub type XsField<T> = crate::Reported<T>;
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CrossSection {
    /// Source file, `Geometry/Cross Sections/Attributes` and the row of this section.
    pub id: super::Identity,
    pub river: String,
    pub reach: String,
    pub station: String,
    pub name: XsField<String>,
    pub left_bank: XsField<f64>,
    pub right_bank: XsField<f64>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CrossSectionGeometry {
    pub source: PathBuf,
    pub dataset: String,
    pub sections: Vec<CrossSection>,
}
/// Deprecated name of the shared [`super::ResultMode`].
#[deprecated(note = "use hdf::ResultMode")]
pub type XsResultKind = super::ResultMode;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum XsVariable {
    WaterSurface,
    Flow,
}
impl XsVariable {
    fn name(self) -> &'static str {
        match self {
            Self::WaterSurface => "Water Surface",
            Self::Flow => "Flow",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanResultMode {
    Steady,
    Unsteady,
    Mixed,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResultFamily {
    Mesh2D,
    CrossSectionsUnsteady,
    SteadyProfiles,
    Structures,
    Pipes,
    ReferenceLines,
    ReferencePoints,
    BoundaryConditions,
    CalibrationRegions,
}
impl PlanResultMode {
    /// Stable snake_case spelling (never the Debug form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steady => "steady",
            Self::Unsteady => "unsteady",
            Self::Mixed => "mixed",
            Self::Unavailable => "unavailable",
        }
    }
}
impl ResultFamily {
    /// Stable snake_case spelling (never the Debug form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mesh2D => "mesh_2d",
            Self::CrossSectionsUnsteady => "cross_sections_unsteady",
            Self::SteadyProfiles => "steady_profiles",
            Self::Structures => "structures",
            Self::Pipes => "pipes",
            Self::ReferenceLines => "reference_lines",
            Self::ReferencePoints => "reference_points",
            Self::BoundaryConditions => "boundary_conditions",
            Self::CalibrationRegions => "calibration_regions",
        }
    }
}
pub type FamilyStatus = crate::Capability;
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FamilyCapability {
    pub family: ResultFamily,
    pub status: FamilyStatus,
    pub datasets: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlanCapabilities {
    pub mode: PlanResultMode,
    pub families: Vec<FamilyCapability>,
    /// Per qualified structure family (SA 2D connections, lateral structures, storage areas).
    pub structures: Vec<super::structures::StructureCapability>,
}
fn corrupt(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt, message)
}
fn unsupported(code: UnsupportedCode, message: impl Into<String>) -> Error {
    Error::unsupported(code, message)
}
/// Every identity row must equal the geometry section at the same position (trimmed).
fn compare_identity(
    identities: &[[String; 3]],
    geometry: &CrossSectionGeometry,
    source: &str,
) -> Result<()> {
    if identities.len() != geometry.sections.len() {
        return Err(corrupt(format!("{source} identity count mismatch")));
    }
    for (i, (row, xs)) in identities.iter().zip(&geometry.sections).enumerate() {
        if row[0].trim() != xs.river.trim()
            || row[1].trim() != xs.reach.trim()
            || row[2].trim() != xs.station.trim()
        {
            return Err(corrupt(format!(
                "{source} XS identity mismatch at column {i}"
            )));
        }
    }
    Ok(())
}
fn base(kind: ResultMode) -> &'static str {
    match kind {
        ResultMode::Steady => STEADY,
        ResultMode::Unsteady => UNSTEADY,
    }
}
impl ReadOnlyHdfSession {
    pub(super) fn object_kind(&self, path: &str) -> Result<Option<bool>> {
        let mut current = String::new();
        let parts: Vec<_> = path.trim_matches('/').split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            let members = self.group_members(if current.is_empty() { "/" } else { &current })?;
            if members.datasets.iter().any(|n| n == part) {
                if i + 1 != parts.len() {
                    return Err(corrupt(format!(
                        "dataset used as group at {current}/{part}"
                    )));
                }
                return Ok(Some(false));
            }
            if !members.groups.iter().any(|n| n == part) {
                return Ok(None);
            }
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
        }
        Ok(Some(true))
    }
    pub(super) fn has_group(&self, path: &str) -> Result<bool> {
        match self.object_kind(path)? {
            None => Ok(false),
            Some(true) => Ok(true),
            Some(false) => Err(corrupt(format!("expected group at {path}"))),
        }
    }
    pub(super) fn has_dataset(&self, path: &str) -> Result<bool> {
        match self.object_kind(path)? {
            None => Ok(false),
            Some(false) => Ok(true),
            Some(true) => Err(corrupt(format!("expected dataset at {path}"))),
        }
    }
    /// Unlike Python's membership check, a dataset at Results/Steady is Corrupt.
    /// Presence of a valid group does not certify completed results.
    pub fn is_steady_plan(&self) -> Result<bool> {
        self.has_group("Results/Steady")
    }
    pub fn plan_result_mode(&self) -> Result<PlanResultMode> {
        Ok(
            match (self.is_steady_plan()?, self.has_group("Results/Unsteady")?) {
                (true, true) => PlanResultMode::Mixed,
                (true, false) => PlanResultMode::Steady,
                (false, true) => PlanResultMode::Unsteady,
                (false, false) => PlanResultMode::Unavailable,
            },
        )
    }
    pub fn cross_sections(&self, max_bytes: usize) -> Result<CrossSectionGeometry> {
        self.memo(format!("cross_sections:{:?}", (max_bytes,)), || {
            self.has_dataset(GEOMETRY)?;
            let ds = self.file.dataset(GEOMETRY).map_err(map_error)?;
            if ds.shape().first().copied().unwrap_or(0) > 100_000 {
                return Err(unsupported(
                    UnsupportedCode::ResourceLimit,
                    "cross-section descriptor count exceeds 100000",
                ));
            }
            let table = self.simple_compound(GEOMETRY, max_bytes)?;
            if table.shape.len() != 1 {
                return Err(corrupt("cross-section attributes must be rank one"));
            }
            let count = usize::try_from(table.shape[0])
                .map_err(|_| corrupt("cross-section count overflow"))?;
            if count > 100_000 {
                return Err(unsupported(
                    UnsupportedCode::ResourceLimit,
                    "cross-section descriptor count exceeds 100000",
                ));
            }
            for name in ["River", "Reach", "RS"] {
                if !table.columns.iter().any(|c| c.name == name) {
                    return Err(unsupported(
                        UnsupportedCode::Schema,
                        format!("missing cross-section identity field {name}"),
                    ));
                }
            }
            let column = |name: &str| table.columns.iter().find(|c| c.name == name);
            let text = |name: &str, i: usize| -> Result<XsField<String>> {
                Ok(match column(name) {
                    None => XsField::unavailable(format!("missing cross-section field {name}")),
                    Some(c) => match &c.value {
                        MetadataValue::Strings(v) => XsField::supported(
                            v.get(i)
                                .ok_or_else(|| corrupt("cross-section column length mismatch"))?
                                .clone(),
                        ),
                        MetadataValue::Unsupported { code, reason } => {
                            XsField::unsupported(*code, reason.clone())
                        }
                        MetadataValue::Corrupt { reason } => XsField::corrupt(reason.clone()),
                        _ => XsField::unsupported(
                            UnsupportedCode::Encoding,
                            format!("{name} is not proven fixed text"),
                        ),
                    },
                })
            };
            let bank = |name: &str, i: usize| -> Result<XsField<f64>> {
                Ok(match column(name) {
                    None => XsField::unavailable(format!("missing cross-section field {name}")),
                    Some(c) => match &c.value {
                        MetadataValue::Float(v) => XsField::supported(
                            *v.get(i)
                                .ok_or_else(|| corrupt("cross-section bank length mismatch"))?,
                        ),
                        MetadataValue::Unsupported { code, reason } => {
                            XsField::unsupported(*code, reason.clone())
                        }
                        MetadataValue::Corrupt { reason } => XsField::corrupt(reason.clone()),
                        _ => XsField::unsupported(
                            UnsupportedCode::Encoding,
                            format!("{name} is not proven floating point"),
                        ),
                    },
                })
            };
            let required = |name: &str, i| -> Result<String> {
                let field = text(name, i)?;
                match (field.value(), field.capability()) {
                    (Some(v), _) if !v.trim().is_empty() => Ok(v.clone()),
                    (Some(_), _) => Err(corrupt(format!("empty cross-section identity {name}"))),
                    (None, Capability::Unavailable { reason }) => {
                        Err(unsupported(UnsupportedCode::Schema, reason.clone()))
                    }
                    (None, Capability::Unsupported { code, reason }) => {
                        Err(unsupported(*code, reason.clone()))
                    }
                    (None, cap) => Err(corrupt(cap.reason().unwrap_or("corrupt identity field"))),
                }
            };
            let mut sections = Vec::with_capacity(count);
            for i in 0..count {
                sections.push(CrossSection {
                    id: super::Identity::new(self.path(), GEOMETRY, i as u64),
                    river: required("River", i)?,
                    reach: required("Reach", i)?,
                    station: required("RS", i)?,
                    name: text("Name", i)?,
                    left_bank: bank("Left Bank", i)?,
                    right_bank: bank("Right Bank", i)?,
                });
            }
            let mut identities = std::collections::HashSet::new();
            for xs in &sections {
                if !identities.insert((xs.river.trim(), xs.reach.trim(), xs.station.trim())) {
                    return Err(corrupt("duplicate trimmed cross-section identity"));
                }
            }
            Ok(CrossSectionGeometry {
                source: self.path().to_owned(),
                dataset: GEOMETRY.into(),
                sections,
            })
        })
    }
    /// Proven fixed-string dataset only; vlen/reference profiles are unsupported.
    pub fn steady_profile_names(&self, max_bytes: usize) -> Result<Vec<String>> {
        self.memo(format!("steady_profile_names:{:?}", (max_bytes,)), || {
            let path = format!("{STEADY}/Profile Names");
            let ds = self.file.dataset(&path).map_err(map_error)?;
            if ds.shape().len() != 1 {
                return Err(corrupt("profile names must be rank one"));
            }
            if ds.raw_byte_len().map_err(map_error)? > max_bytes.min(MAX_NAME_TABLE_BYTES) {
                return Err(unsupported(
                    UnsupportedCode::ResourceLimit,
                    "profile names exceed max_bytes or the 8 MiB name-table cap",
                ));
            }
            let p = proof::proof(&ds.raw_datatype().map_err(map_error)?)?;
            if let proof::Kind::Compound(fields) = &p.kind {
                return Err(unsupported(
                    UnsupportedCode::Schema,
                    format!("profile names are compound ({} fields)", fields.len()),
                ));
            }
            match proof::decode(&p, &ds.read_raw_bytes().map_err(map_error)?, ds.shape()[0])? {
                MetadataValue::Strings(v)
                    if !v.is_empty() && v.iter().all(|s| !s.trim().is_empty()) =>
                {
                    let mut seen = std::collections::HashSet::new();
                    if v.iter().any(|name| !seen.insert(name.trim())) {
                        return Err(corrupt("duplicate steady profile names"));
                    }
                    Ok(v)
                }
                MetadataValue::Strings(_) => Err(corrupt("empty steady profile names")),
                MetadataValue::Unsupported { code, reason } => Err(unsupported(code, reason)),
                MetadataValue::Corrupt { reason } => Err(corrupt(reason)),
                _ => Err(unsupported(
                    UnsupportedCode::Encoding,
                    "profile names are not fixed text",
                )),
            }
        })
    }
    fn proven_strings(&self, path: &str, max_bytes: usize) -> Result<Vec<String>> {
        self.has_dataset(path)?;
        let ds = self.file.dataset(path).map_err(map_error)?;
        if ds.shape().len() != 1 {
            return Err(corrupt("result identity table must be rank one"));
        }
        if ds.raw_byte_len().map_err(map_error)? > max_bytes.min(MAX_NAME_TABLE_BYTES) {
            return Err(unsupported(
                UnsupportedCode::ResourceLimit,
                "result identity table exceeds max_bytes or the 8 MiB name-table cap",
            ));
        }
        let p = proof::proof(&ds.raw_datatype().map_err(map_error)?)?;
        match proof::decode(&p, &ds.read_raw_bytes().map_err(map_error)?, ds.shape()[0])? {
            MetadataValue::Strings(v) => Ok(v),
            _ => Err(unsupported(
                UnsupportedCode::Encoding,
                "result identity table is not proven fixed text",
            )),
        }
    }
    pub(super) fn validate_identity(
        &self,
        kind: ResultMode,
        geometry: &CrossSectionGeometry,
        max_bytes: usize,
    ) -> Result<()> {
        let path = match kind {
            ResultMode::Steady => {
                "Results/Steady/Output/Geometry Info/Cross Section Attributes".into()
            }
            ResultMode::Unsteady => format!("{UNSTEADY}/Cross Sections/Cross Section Attributes"),
        };
        let identities = if self.has_dataset(&path)? {
            let t = self.simple_compound(&path, max_bytes)?;
            if t.shape != vec![geometry.sections.len() as u64] {
                return Err(corrupt("results-side identity count mismatch"));
            }
            let mut columns: Vec<&Vec<String>> = Vec::new();
            for key in ["River", "Reach", "Station"] {
                let c = t.columns.iter().find(|c| c.name == key).ok_or_else(|| {
                    unsupported(
                        UnsupportedCode::IdentityLayout,
                        format!("results-side identity field {key} missing"),
                    )
                })?;
                match &c.value {
                    MetadataValue::Strings(v) if v.len() == geometry.sections.len() => {
                        columns.push(v)
                    }
                    MetadataValue::Strings(_) => {
                        return Err(corrupt(format!(
                            "results-side identity field {key} length disagrees with the table"
                        )))
                    }
                    _ => {
                        return Err(unsupported(
                            UnsupportedCode::Encoding,
                            format!("results-side identity field {key} unproven"),
                        ))
                    }
                }
            }
            (0..geometry.sections.len())
                .map(|i| {
                    [
                        columns[0][i].clone(),
                        columns[1][i].clone(),
                        columns[2][i].clone(),
                    ]
                })
                .collect::<Vec<_>>()
        } else {
            let path = match kind {
                ResultMode::Steady => {
                    "Results/Steady/Output/Geometry Info/Cross Section Only".into()
                }
                ResultMode::Unsteady => format!("{UNSTEADY}/Cross Sections/Cross Section Only"),
            };
            if !self.has_dataset(&path)? {
                return match kind {
                    // Python `HdfResultsPlan._get_steady_cross_section_attributes` falls back
                    // to the geometry table; several HEC-RAS 6.x steady plans write no
                    // results-side identity.
                    ResultMode::Steady => {
                        let rows = self.steady_geometry_identity(max_bytes)?;
                        compare_identity(&rows, geometry, "geometry-fallback")
                    }
                    // Python's unsteady readers (`HdfResultsXsec`) require the results-side
                    // `Cross Section Attributes`; there is no geometry fallback to mirror.
                    ResultMode::Unsteady => Err(unsupported(
                        UnsupportedCode::IdentityLayout,
                        "results-side identity table absent",
                    )),
                };
            }
            self.proven_strings(&path, max_bytes)?
                .into_iter()
                .map(|s| {
                    let b = s.as_bytes();
                    if !s.is_ascii()
                        || b.len() < 34
                        || b.len() > 42
                        || b.get(16) != Some(&b' ')
                        || b.get(33) != Some(&b' ')
                    {
                        return Err(unsupported(
                            UnsupportedCode::IdentityLayout,
                            "unproven packed XS identity layout (16/16/8 plus separators)",
                        ));
                    }
                    Ok([s[..16].into(), s[17..33].into(), s[34..].into()])
                })
                .collect::<Result<Vec<_>>>()?
        };
        compare_identity(&identities, geometry, "results-side")
    }
    /// Steady identity when no results-side table (`Cross Section Attributes` or the packed
    /// `Cross Section Only`) exists: `Geometry/Cross Sections/Attributes`, selected exactly as
    /// Python `HdfResultsPlan._get_steady_cross_section_attributes` does. Fields are looked up
    /// by lower-cased name (`river`, `reach`, then `station` or else `rs`); results align by
    /// order and the count must equal the result column count (checked against the geometry
    /// count, which `xs_dataset` ties to the result axis). Unlike Python, the selected fields
    /// must be proven fixed text, and a name that is ambiguous by case is refused instead of
    /// letting the last field win.
    fn steady_geometry_identity(&self, max_bytes: usize) -> Result<Vec<[String; 3]>> {
        if !self.has_dataset(GEOMETRY)? {
            return Err(unsupported(
                UnsupportedCode::IdentityLayout,
                "steady identity absent: no results-side table and no Geometry/Cross Sections/Attributes",
            ));
        }
        let t = self.simple_compound(GEOMETRY, max_bytes)?;
        if t.shape.len() != 1 {
            return Err(corrupt("geometry-fallback identity table must be rank one"));
        }
        let count = usize::try_from(t.shape[0])
            .map_err(|_| corrupt("geometry-fallback identity count overflow"))?;
        let lookup = |key: &str| -> Result<Option<&super::metadata::CompoundColumn>> {
            let mut hits = t.columns.iter().filter(|c| c.name.to_lowercase() == key);
            let first = hits.next();
            if hits.next().is_some() {
                return Err(unsupported(
                    UnsupportedCode::IdentityLayout,
                    format!("geometry-fallback identity field {key} is ambiguous by case"),
                ));
            }
            Ok(first)
        };
        let station = match lookup("station")? {
            Some(c) => Some(c),
            None => lookup("rs")?,
        };
        let mut columns: Vec<&Vec<String>> = Vec::new();
        for (key, column) in [
            ("river", lookup("river")?),
            ("reach", lookup("reach")?),
            ("station", station),
        ] {
            let c = column.ok_or_else(|| {
                unsupported(
                    UnsupportedCode::IdentityLayout,
                    format!("geometry-fallback identity field {key} missing"),
                )
            })?;
            match &c.value {
                MetadataValue::Strings(v) if v.len() == count => columns.push(v),
                MetadataValue::Strings(_) => {
                    return Err(corrupt(format!(
                        "geometry-fallback identity field {} length disagrees with the table",
                        c.name
                    )))
                }
                MetadataValue::Unsupported { code, reason } => {
                    return Err(unsupported(
                        *code,
                        format!("geometry-fallback identity field {}: {reason}", c.name),
                    ))
                }
                MetadataValue::Corrupt { reason } => {
                    return Err(corrupt(format!(
                        "geometry-fallback identity field {}: {reason}",
                        c.name
                    )))
                }
                _ => {
                    return Err(unsupported(
                        UnsupportedCode::Encoding,
                        format!(
                            "geometry-fallback identity field {} is not proven fixed text",
                            c.name
                        ),
                    ))
                }
            }
        }
        Ok((0..count)
            .map(|i| {
                [
                    columns[0][i].clone(),
                    columns[1][i].clone(),
                    columns[2][i].clone(),
                ]
            })
            .collect())
    }
    fn xs_dataset(
        &self,
        kind: ResultMode,
        variable: XsVariable,
        geometry: &CrossSectionGeometry,
        profiles: Option<&[String]>,
    ) -> Result<String> {
        let path = format!("{}/Cross Sections/{}", base(kind), variable.name());
        let cap = self.numeric_capability(&path)?;
        if !matches!(cap.kind, Some(NumericKind::F32 | NumericKind::F64)) {
            return Err(unsupported(
                UnsupportedCode::Encoding,
                "XS result dtype must be proven f32/f64",
            ));
        }
        if cap.shape.len() != 2 || cap.shape[1] != geometry.sections.len() as u64 {
            return Err(corrupt(
                "XS results must have (time/profile, cross-section) shape",
            ));
        }
        if let Some(profiles) = profiles {
            if cap.shape[0] != profiles.len() as u64 {
                return Err(corrupt("steady profile/result row count mismatch"));
            }
        }
        if cap.shape[0] == 0 || cap.shape[1] == 0 {
            return Err(unsupported(UnsupportedCode::Schema, "empty XS result axes"));
        }
        if kind == ResultMode::Unsteady {
            // The base time axis, when stored, must have one entry per result row (checked on
            // the dataset shape, as structure reads do, without parsing time text). Reads and
            // capability reports share this check.
            let time_path = format!("{UNSTEADY}/Time");
            if let Some(time) = absent_ok(self.file.dataset(&time_path).map_err(map_error))? {
                if time.shape() != [cap.shape[0]] {
                    return Err(corrupt(format!(
                        "{path}: {} rows but the unsteady Time axis has {:?}",
                        cap.shape[0],
                        time.shape()
                    )));
                }
            }
        }
        Ok(path)
    }
    /// Exact F32/F64 selected results; max_bytes bounds returned bytes, not HDF chunk inflation.
    pub fn read_cross_section_result(
        &self,
        kind: ResultMode,
        variable: XsVariable,
        rows: Slice,
        sections: Slice,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<NumericArray> {
        let geometry = self.cross_sections(metadata_max_bytes)?;
        self.validate_identity(kind, &geometry, metadata_max_bytes)?;
        let profiles = if kind == ResultMode::Steady {
            Some(self.steady_profile_names(metadata_max_bytes)?)
        } else {
            None
        };
        let path = self.xs_dataset(kind, variable, &geometry, profiles.as_deref())?;
        let cap = self.numeric_capability(&path)?;
        let mut elements = 1u64;
        for (i, s) in [rows, sections].iter().enumerate() {
            if s.step == 0 || s.start > s.end || s.end > cap.shape[i] {
                return Err(Error::new(
                    ErrorKind::InvalidRequest,
                    "invalid XS result selection",
                ));
            }
            let n = (s.end - s.start).div_ceil(s.step);
            elements = elements.checked_mul(n).ok_or_else(|| {
                Error::new(ErrorKind::InvalidRequest, "XS selection size overflow")
            })?;
        }
        let width = if cap.kind == Some(NumericKind::F64) {
            8
        } else {
            4
        };
        if elements
            .checked_mul(width)
            .is_none_or(|n| n > max_bytes as u64)
        {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "XS selected result exceeds max_bytes",
            ));
        }
        self.read_numeric(&path, Some(&[rows, sections]))
    }
    pub fn plan_capabilities(&self, max_bytes: usize) -> Result<PlanCapabilities> {
        self.memo(format!("plan_capabilities:{:?}", (max_bytes,)), || {
            let mut families = Vec::new();
            let mut geometry = None;
            for (family, kind) in [
                (ResultFamily::CrossSectionsUnsteady, ResultMode::Unsteady),
                (ResultFamily::SteadyProfiles, ResultMode::Steady),
            ] {
                let mut datasets = Vec::new();
                let mut errors = Vec::new();
                for variable in [XsVariable::WaterSurface, XsVariable::Flow] {
                    let path = format!("{}/Cross Sections/{}", base(kind), variable.name());
                    match self.has_dataset(&path) {
                        Ok(true) => datasets.push((path, variable)),
                        Ok(false) => {}
                        Err(e) => errors.push(e),
                    }
                }
                if !datasets.is_empty() {
                    let g = geometry.get_or_insert_with(|| self.cross_sections(max_bytes));
                    let validation = (|| {
                        let g = g.as_ref().map_err(Clone::clone)?;
                        self.validate_identity(kind, g, max_bytes)?;
                        let profiles = if kind == ResultMode::Steady {
                            Some(self.steady_profile_names(max_bytes)?)
                        } else {
                            None
                        };
                        for (path, var) in &datasets {
                            if let Err(mut e) = self.xs_dataset(kind, *var, g, profiles.as_deref())
                            {
                                e.message = format!("{path}: {}", e.message);
                                errors.push(e);
                            }
                        }
                        Ok::<_, Error>(())
                    })();
                    if let Err(e) = validation {
                        errors.push(e);
                    }
                }
                let status = match errors_to_capability(errors)? {
                    Some(c) => c,
                    None if datasets.is_empty() => {
                        FamilyStatus::unavailable("canonical XS result datasets absent")
                    }
                    None => FamilyStatus::Supported,
                };
                families.push(FamilyCapability {
                    family,
                    status,
                    datasets: datasets.into_iter().map(|(p, _)| p).collect(),
                });
            }
            let mesh = format!("{UNSTEADY}/2D Flow Areas");
            families.push(FamilyCapability {
                family: ResultFamily::Mesh2D,
                status: match self.has_group(&mesh) {
                    Ok(true) => FamilyStatus::delegated("use existing per-mesh readers"),
                    Ok(false) => FamilyStatus::unavailable("2D result group absent"),
                    Err(e) => error_to_capability(e)?,
                },
                datasets: Vec::new(),
            });
            let structures = self.structure_capabilities(max_bytes)?;
            families.push(structures.0);
            for (family, names) in [
                (ResultFamily::Pipes, &["Pipes", "Pipe Networks"][..]),
                (ResultFamily::ReferenceLines, &["Reference Lines"][..]),
                (ResultFamily::ReferencePoints, &["Reference Points"][..]),
                (
                    ResultFamily::BoundaryConditions,
                    &["Boundary Conditions"][..],
                ),
                (
                    ResultFamily::CalibrationRegions,
                    &["Calibration Regions"][..],
                ),
            ] {
                if family == ResultFamily::ReferencePoints {
                    let inv = match self.read_family_inventory(
                        super::families::ReadFamily::ReferencePoints,
                        max_bytes,
                    ) {
                        Ok(inv) => inv,
                        Err(e) => {
                            families.push(FamilyCapability {
                                family,
                                status: error_to_capability(e)?,
                                datasets: Vec::new(),
                            });
                            continue;
                        }
                    };
                    if !inv.datasets.is_empty() {
                        let status = inv
                            .datasets
                            .iter()
                            .map(|d| &d.stored.capability)
                            .find(|c| matches!(c, crate::Capability::Corrupt { .. }))
                            .or_else(|| {
                                inv.datasets
                                    .iter()
                                    .map(|d| &d.stored.capability)
                                    .find(|c| !c.is_supported())
                            })
                            .cloned()
                            .unwrap_or(inv.capability);
                        families.push(FamilyCapability {
                            family,
                            status,
                            datasets: inv.datasets.into_iter().map(|d| d.stored.path).collect(),
                        });
                        continue;
                    }
                }
                let mut found = Vec::new();
                let mut failures = Vec::new();
                for root in [UNSTEADY, STEADY] {
                    for name in names {
                        let p = format!("{root}/{name}");
                        match self.object_kind(&p) {
                            Ok(Some(true)) => found.push(p),
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
                }
                families.push(FamilyCapability {
                    family,
                    status: if !failures.is_empty() {
                        FamilyStatus::corrupt(failures.join("; "))
                    } else if found.is_empty() {
                        FamilyStatus::unavailable(
                            "canonical family objects absent; other layouts unsupported",
                        )
                    } else {
                        match family {
                            ResultFamily::Pipes => FamilyStatus::unsupported(
                                UnsupportedCode::NotImplemented,
                                format!("typed extraction unimplemented: {}", found.join(", ")),
                            ),
                            // No real fixture contains these layouts (or a trivial layout is
                            // unproven): presence is reported, never read or guessed.
                            _ => FamilyStatus::unsupported(
                                UnsupportedCode::IdentityLayout,
                                format!("no qualified real layout: {}", found.join(", ")),
                            ),
                        }
                    },
                    datasets: found,
                });
            }
            Ok(PlanCapabilities {
                mode: self.plan_result_mode()?,
                families,
                structures: structures.1,
            })
        })
    }
}
