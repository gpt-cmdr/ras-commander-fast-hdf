//! Qualified packed line/polygon families. Offsets and coordinates remain source values.
use super::{
    mesh_geometry::{corrupt, exact_compound, selected, ExactCompound},
    metadata::MetadataValue,
    Identity, NumericArray, NumericValues, ReadOnlyHdfSession,
};
use crate::{Capability, Error, ErrorKind, Reported, Result, UnsupportedCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeometryFamily {
    BoundaryConditions,
    Breaklines,
    RefinementRegions,
    ReferenceLines,
    ReferencePoints,
    CalibrationRegions,
}
pub const GEOMETRY_FAMILIES: &[GeometryFamily] = &[
    GeometryFamily::BoundaryConditions,
    GeometryFamily::Breaklines,
    GeometryFamily::RefinementRegions,
    GeometryFamily::ReferenceLines,
    GeometryFamily::ReferencePoints,
    GeometryFamily::CalibrationRegions,
];
impl GeometryFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BoundaryConditions => "boundary_conditions",
            Self::Breaklines => "breaklines",
            Self::RefinementRegions => "refinement_regions",
            Self::ReferenceLines => "reference_lines",
            Self::ReferencePoints => "reference_points",
            Self::CalibrationRegions => "calibration_regions",
        }
    }
    pub fn group_path(self) -> &'static str {
        match self {
            Self::BoundaryConditions => "Geometry/Boundary Condition Lines",
            Self::Breaklines => "Geometry/2D Flow Area Break Lines",
            Self::RefinementRegions => "Geometry/2D Flow Area Refinement Regions",
            Self::ReferenceLines => "Geometry/Reference Lines",
            Self::ReferencePoints => "Geometry/Reference Points",
            Self::CalibrationRegions => "Geometry/Land Cover (Manning's n)",
        }
    }
    pub(super) fn prefix(self) -> &'static str {
        if matches!(self, Self::RefinementRegions | Self::CalibrationRegions) {
            "Polygon"
        } else {
            "Polyline"
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GeometryFeature {
    pub id: Identity,
    pub name: String,
    pub name_capability: Capability,
    /// Range in the shared exact points array, from Info columns 0/1.
    pub point_range: std::ops::Range<u64>,
    /// Ranges in the shared points array. Parts offsets are relative to this feature.
    pub parts: Vec<std::ops::Range<u64>>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct GeometryFamilyData {
    pub family: GeometryFamily,
    pub attributes: ExactCompound,
    pub info: NumericArray,
    pub parts: NumericArray,
    pub points: NumericArray,
    pub features: Vec<GeometryFeature>,
    /// Exact optional association/calibration tables, preserving column names and raw bytes.
    pub tables: Vec<(String, ExactCompound)>,
    pub terrain_profiles: Vec<(String, NumericArray)>,
}
fn integers(a: &NumericArray) -> Result<&[i32]> {
    if let NumericValues::I32(v) = &a.values {
        Ok(v)
    } else {
        Err(Error::unsupported(
            UnsupportedCode::Encoding,
            "packed geometry offsets require proven i32",
        ))
    }
}
fn range(start: i32, count: i32, bound: u64) -> Result<std::ops::Range<u64>> {
    let start = u64::try_from(start).map_err(|_| corrupt("negative packed geometry offset"))?;
    let count = u64::try_from(count).map_err(|_| corrupt("negative packed geometry count"))?;
    let end = start
        .checked_add(count)
        .filter(|&end| end <= bound)
        .ok_or_else(|| corrupt("packed geometry range outside array"))?;
    Ok(start..end)
}
impl ReadOnlyHdfSession {
    /// One family, exact packed arrays plus row identities and validated coordinate parts.
    /// `max_bytes` bounds each consulted dataset. No geometry repair or ring closing.
    pub fn geometry_family(
        &self,
        family: GeometryFamily,
        max_bytes: usize,
    ) -> Result<Reported<GeometryFamilyData>> {
        // Canonical absence, then every dataset read below (see `presence`).
        if let Some(reason) = self.geometry_family_prelude(family)? {
            return Ok(Reported::unavailable(reason));
        }
        let group = family.group_path();
        if family == GeometryFamily::ReferencePoints {
            return Ok(Reported::unsupported(
                UnsupportedCode::NotImplemented,
                "point geometry is not packed lines; use reference_point_geometry",
            ));
        }
        let prefix = family.prefix();
        let attr_path = format!("{group}/Attributes");
        let attributes = exact_compound(self, &attr_path, max_bytes)?;
        let names = attributes
            .table
            .columns
            .iter()
            .find(|c| c.name == "Name")
            .ok_or_else(|| {
                Error::unsupported(
                    UnsupportedCode::IdentityLayout,
                    "geometry Name column absent",
                )
            })?;
        let names = match &names.value {
            MetadataValue::Strings(v) => v,
            _ => {
                return Err(Error::unsupported(
                    UnsupportedCode::Encoding,
                    "geometry names require proven fixed text",
                ))
            }
        };
        if attributes.table.shape != [names.len() as u64] {
            return Err(corrupt("geometry Attributes must be rank one"));
        }
        let info = selected(self, &format!("{group}/{prefix} Info"), None, max_bytes)?;
        let parts = selected(self, &format!("{group}/{prefix} Parts"), None, max_bytes)?;
        let points = selected(self, &format!("{group}/{prefix} Points"), None, max_bytes)?;
        if info.shape != [names.len() as u64, 4]
            || parts.shape.len() != 2
            || parts.shape[1] != 2
            || points.shape.len() != 2
            || points.shape[1] != 2
        {
            return Err(corrupt(
                "invalid packed geometry shape or Attributes/Info row count",
            ));
        }
        if !matches!(points.values, NumericValues::F32(_) | NumericValues::F64(_)) {
            return Err(Error::unsupported(
                UnsupportedCode::Encoding,
                "geometry coordinates must be f32/f64",
            ));
        }
        let iv = integers(&info)?;
        let pv = integers(&parts)?;
        let mut features = Vec::new();
        for (row, name) in names.iter().enumerate() {
            let i = &iv[row * 4..row * 4 + 4];
            let point_range = range(i[0], i[1], points.shape[0])?;
            let part_range = range(i[2], i[3], parts.shape[0])?;
            let mut ranges = Vec::new();
            for p in part_range {
                let p = usize::try_from(p).map_err(|_| corrupt("part index exceeds usize"))?;
                let r = range(
                    pv[p * 2],
                    pv[p * 2 + 1],
                    point_range.end - point_range.start,
                )?;
                ranges.push(point_range.start + r.start..point_range.start + r.end);
            }
            features.push(GeometryFeature {
                id: Identity::new(self.path(), &attr_path, row as u64),
                name: name.clone(),
                name_capability: if name.trim().is_empty() {
                    Capability::corrupt("empty geometry feature name")
                } else {
                    Capability::Supported
                },
                point_range,
                parts: ranges,
            });
        }
        let mut tables = Vec::new();
        for name in [
            "External Faces",
            "Internal Faces",
            "Internal Cells",
            "Calibration Table",
        ] {
            let path = format!("{group}/{name}");
            if self.has_dataset(&path)? {
                tables.push((name.into(), exact_compound(self, &path, max_bytes)?));
            }
        }
        let mut terrain_profiles = Vec::new();
        if family == GeometryFamily::ReferenceLines {
            for name in ["Terrain Profiles Info", "Terrain Profiles Values"] {
                let path = format!("{group}/{name}");
                if self.has_dataset(&path)? {
                    terrain_profiles.push((name.into(), selected(self, &path, None, max_bytes)?));
                }
            }
        }
        Ok(Reported::supported(GeometryFamilyData {
            family,
            attributes,
            info,
            parts,
            points,
            features,
            tables,
            terrain_profiles,
        }))
    }
    /// Isolated family status; host I/O failures propagate, malformed storage stays Corrupt.
    pub fn geometry_family_capability(
        &self,
        family: GeometryFamily,
        max_bytes: usize,
    ) -> Result<Capability> {
        if family == GeometryFamily::ReferencePoints {
            return match self.reference_point_geometry(max_bytes) {
                Ok(v) => Ok(v.capability().clone()),
                Err(e) if e.kind == ErrorKind::NotFound => Ok(Capability::corrupt(e.message)),
                Err(e) => crate::capability::error_to_capability(e),
            };
        }
        match self.geometry_family(family, max_bytes) {
            Ok(v) => Ok(v.capability().clone()),
            Err(e) if e.kind == ErrorKind::NotFound => Ok(Capability::corrupt(e.message)),
            Err(e) => crate::capability::error_to_capability(e),
        }
    }
}
