//! Exact stored mesh arrays. Selection is on axis zero, including stored ghost rows.
use super::{
    metadata::CompoundTable, NumericArray, NumericCapability, NumericKind, ReadOnlyHdfSession,
    Slice,
};
use crate::{Capability, Error, ErrorKind, Reported, Result, UnsupportedCode};

const ROOT: &str = "Geometry/2D Flow Areas";

/// A qualified canonical mesh geometry dataset, never a caller-supplied HDF path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MeshGeometryField {
    CellCenters,
    FaceMinimumElevation,
    CellManningsN,
    FacePointIsPerimeter,
    CellMinimumElevation,
    FacePoints,
    FacePointIndexes,
    FaceCellIndexes,
    FaceNormalsAndLength,
    FacePerimeterInfo,
    FacePerimeterValues,
    CellFaceInfo,
    CellFaceValues,
    CellFacePointIndexes,
    Perimeter,
    FaceAreaElevationInfo,
    FaceAreaElevationValues,
    CellVolumeElevationInfo,
    CellVolumeElevationValues,
}
pub const MESH_GEOMETRY_FIELDS: &[MeshGeometryField] = &[
    MeshGeometryField::FaceMinimumElevation,
    MeshGeometryField::CellManningsN,
    MeshGeometryField::FacePointIsPerimeter,
    MeshGeometryField::CellCenters,
    MeshGeometryField::CellMinimumElevation,
    MeshGeometryField::FacePoints,
    MeshGeometryField::FacePointIndexes,
    MeshGeometryField::FaceCellIndexes,
    MeshGeometryField::FaceNormalsAndLength,
    MeshGeometryField::FacePerimeterInfo,
    MeshGeometryField::FacePerimeterValues,
    MeshGeometryField::CellFaceInfo,
    MeshGeometryField::CellFaceValues,
    MeshGeometryField::CellFacePointIndexes,
    MeshGeometryField::Perimeter,
    MeshGeometryField::FaceAreaElevationInfo,
    MeshGeometryField::FaceAreaElevationValues,
    MeshGeometryField::CellVolumeElevationInfo,
    MeshGeometryField::CellVolumeElevationValues,
];
impl MeshGeometryField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FaceMinimumElevation => "face_minimum_elevation",
            Self::CellManningsN => "cell_mannings_n",
            Self::FacePointIsPerimeter => "face_point_is_perimeter",
            Self::CellCenters => "cell_centers",
            Self::CellMinimumElevation => "cell_minimum_elevation",
            Self::FacePoints => "face_points",
            Self::FacePointIndexes => "face_point_indexes",
            Self::FaceCellIndexes => "face_cell_indexes",
            Self::FaceNormalsAndLength => "face_normals_and_length",
            Self::FacePerimeterInfo => "face_perimeter_info",
            Self::FacePerimeterValues => "face_perimeter_values",
            Self::CellFaceInfo => "cell_face_info",
            Self::CellFaceValues => "cell_face_values",
            Self::CellFacePointIndexes => "cell_face_point_indexes",
            Self::Perimeter => "perimeter",
            Self::FaceAreaElevationInfo => "face_area_elevation_info",
            Self::FaceAreaElevationValues => "face_area_elevation_values",
            Self::CellVolumeElevationInfo => "cell_volume_elevation_info",
            Self::CellVolumeElevationValues => "cell_volume_elevation_values",
        }
    }
    pub fn dataset_name(self) -> &'static str {
        match self {
            Self::FaceMinimumElevation => "Faces Minimum Elevation",
            Self::CellManningsN => "Cells Center Manning's n",
            Self::FacePointIsPerimeter => "FacePoints Is Perimeter",
            Self::CellCenters => "Cells Center Coordinate",
            Self::CellMinimumElevation => "Cells Minimum Elevation",
            Self::FacePoints => "FacePoints Coordinate",
            Self::FacePointIndexes => "Faces FacePoint Indexes",
            Self::FaceCellIndexes => "Faces Cell Indexes",
            Self::FaceNormalsAndLength => "Faces NormalUnitVector and Length",
            Self::FacePerimeterInfo => "Faces Perimeter Info",
            Self::FacePerimeterValues => "Faces Perimeter Values",
            Self::CellFaceInfo => "Cells Face and Orientation Info",
            Self::CellFaceValues => "Cells Face and Orientation Values",
            Self::CellFacePointIndexes => "Cells FacePoint Indexes",
            Self::Perimeter => "Perimeter",
            Self::FaceAreaElevationInfo => "Faces Area Elevation Info",
            Self::FaceAreaElevationValues => "Faces Area Elevation Values",
            Self::CellVolumeElevationInfo => "Cells Volume Elevation Info",
            Self::CellVolumeElevationValues => "Cells Volume Elevation Values",
        }
    }
    fn integer(self) -> bool {
        matches!(
            self,
            Self::FacePointIsPerimeter
                | Self::FacePointIndexes
                | Self::FaceCellIndexes
                | Self::FacePerimeterInfo
                | Self::CellFaceInfo
                | Self::CellFaceValues
                | Self::CellFacePointIndexes
                | Self::FaceAreaElevationInfo
                | Self::CellVolumeElevationInfo
        )
    }
    fn columns(self) -> Option<u64> {
        match self {
            Self::CellMinimumElevation
            | Self::FaceMinimumElevation
            | Self::CellManningsN
            | Self::FacePointIsPerimeter => None,
            Self::FaceNormalsAndLength => Some(3),
            Self::FaceAreaElevationValues => Some(4),
            Self::CellFacePointIndexes => Some(0), // variable-width padded index rows
            _ => Some(2),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MeshGeometry {
    pub mesh: crate::schema::MeshDescriptor,
    pub fields: Vec<MeshGeometryDataset>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MeshGeometryDataset {
    pub field: MeshGeometryField,
    pub path: String,
    pub shape: Vec<u64>,
    pub kind: Option<NumericKind>,
    pub capability: Capability,
}
/// Compound scalar view plus exact stored bytes (f32/NaN payloads are never lost).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ExactCompound {
    pub table: CompoundTable,
    pub raw_bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MeshGeometryTables {
    pub attributes: ExactCompound,
    /// Observed layouts store numeric (start,count), not a compound.
    pub cell_info: Reported<NumericArray>,
}
pub(super) fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidRequest, message)
}
pub(super) fn corrupt(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt, message)
}
pub(super) fn budget(bytes: usize, max_bytes: usize) -> Result<()> {
    if bytes > max_bytes {
        return Err(invalid("selected data exceeds caller byte limit")
            .with_code(UnsupportedCode::ResourceLimit));
    }
    Ok(())
}
pub(super) fn exact_compound(
    s: &ReadOnlyHdfSession,
    path: &str,
    max_bytes: usize,
) -> Result<ExactCompound> {
    let (table, raw_bytes) = s.simple_compound_with_raw(path, max_bytes)?;
    Ok(ExactCompound { table, raw_bytes })
}
pub(super) fn selected(
    s: &ReadOnlyHdfSession,
    path: &str,
    rows: Option<Slice>,
    max_bytes: usize,
) -> Result<NumericArray> {
    let c = s.numeric_capability(path)?;
    let kind = c
        .kind
        .ok_or_else(|| Error::unsupported(UnsupportedCode::Encoding, "unproven numeric storage"))?;
    let width = match kind {
        NumericKind::I8 | NumericKind::U8 => 1,
        NumericKind::I16 | NumericKind::U16 => 2,
        NumericKind::I32 | NumericKind::U32 | NumericKind::F32 => 4,
        _ => 8,
    };
    let mut selection: Vec<_> = c.shape.iter().map(|&n| Slice::new(0, n, 1)).collect();
    if let Some(rows) = rows {
        if selection.is_empty() {
            return Err(invalid("cannot select rows of scalar"));
        }
        selection[0] = rows;
    }
    let mut count = 1usize;
    for (sl, &dim) in selection.iter().zip(&c.shape) {
        if sl.step == 0 || sl.start > sl.end || sl.end > dim {
            return Err(invalid("invalid element selection"));
        }
        let n = (sl.end - sl.start).div_ceil(sl.step);
        count = usize::try_from(n)
            .ok()
            .and_then(|n| count.checked_mul(n))
            .ok_or_else(|| invalid("selection size overflow"))?;
    }
    let bytes = count
        .checked_mul(width)
        .ok_or_else(|| invalid("selection byte size overflow"))?;
    budget(bytes, max_bytes)?;
    // The low-level reader has no empty-slice support; create a typed empty buffer.
    if count == 0 {
        use super::NumericValues as V;
        let values = match kind {
            NumericKind::I8 => V::I8(vec![]),
            NumericKind::I16 => V::I16(vec![]),
            NumericKind::I32 => V::I32(vec![]),
            NumericKind::I64 => V::I64(vec![]),
            NumericKind::U8 => V::U8(vec![]),
            NumericKind::U16 => V::U16(vec![]),
            NumericKind::U32 => V::U32(vec![]),
            NumericKind::U64 => V::U64(vec![]),
            NumericKind::F32 => V::F32(vec![]),
            NumericKind::F64 => V::F64(vec![]),
        };
        return Ok(NumericArray {
            shape: selection
                .iter()
                .map(|sl| (sl.end - sl.start).div_ceil(sl.step))
                .collect(),
            values,
        });
    }
    s.read_numeric(path, Some(&selection))
}
fn validate(c: &NumericCapability, field: MeshGeometryField, cells: u64) -> Result<()> {
    if !c.capability.is_supported() {
        return Err(Error::unsupported(
            UnsupportedCode::Encoding,
            "unproven mesh array encoding",
        ));
    }
    let float = matches!(c.kind, Some(NumericKind::F32 | NumericKind::F64));
    if float == field.integer() {
        return Err(Error::unsupported(
            UnsupportedCode::Schema,
            "mesh array numeric kind disagrees with domain",
        ));
    }
    let valid = match field.columns() {
        None => c.shape.len() == 1,
        Some(0) => c.shape.len() == 2 && c.shape[1] > 0,
        Some(n) => c.shape.len() == 2 && c.shape[1] == n,
    };
    if !valid {
        return Err(corrupt(format!(
            "invalid shape for {}",
            field.dataset_name()
        )));
    }
    if matches!(
        field,
        MeshGeometryField::CellCenters
            | MeshGeometryField::CellManningsN
            | MeshGeometryField::CellMinimumElevation
            | MeshGeometryField::CellFaceInfo
            | MeshGeometryField::CellFacePointIndexes
            | MeshGeometryField::CellVolumeElevationInfo
    ) && c.shape[0] != cells
    {
        return Err(corrupt("mesh cell array disagrees with stored cell count"));
    }
    Ok(())
}
impl ReadOnlyHdfSession {
    /// Full canonical array inventory; absence is reported per field, never as an empty array.
    pub fn mesh_geometry(&self, area: &str, max_bytes: usize) -> Result<MeshGeometry> {
        self.memo(format!("mesh_geometry:{:?}", (area, max_bytes,)), || {
            if area.is_empty() || area.contains(['/', '\\']) {
                return Err(invalid("area must be one exact mesh name"));
            }
            let mesh = self
                .mesh_inventory(max_bytes)?
                .into_iter()
                .find(|m| m.mesh_name == area)
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "mesh name absent"))?;
            self.mesh_geometry_from_descriptor(mesh)
        })
    }
    pub(crate) fn mesh_geometry_from_descriptor(
        &self,
        mesh: crate::schema::MeshDescriptor,
    ) -> Result<MeshGeometry> {
        let area = &mesh.mesh_name;
        let mut fields = Vec::new();
        for &field in MESH_GEOMETRY_FIELDS {
            let path = format!("{ROOT}/{area}/{}", field.dataset_name());
            let probe = (|| {
                if !self.has_dataset(&path)? {
                    return Ok(None);
                }
                let c = self.numeric_capability(&path)?;
                validate(&c, field, mesh.stored_cells)?;
                Ok(Some(c))
            })();
            let (shape, kind, capability) = match probe {
                Ok(Some(c)) => (c.shape, c.kind, Capability::Supported),
                Ok(None) => (
                    vec![],
                    None,
                    Capability::unavailable("canonical mesh array absent"),
                ),
                Err(e) => (vec![], None, crate::capability::error_to_capability(e)?),
            };
            fields.push(MeshGeometryDataset {
                field,
                path,
                shape,
                kind,
                capability,
            });
        }
        Ok(MeshGeometry { mesh, fields })
    }
    /// Exact stored rows, without converting physical cell indices or dropping ghost rows.
    pub fn read_mesh_geometry(
        &self,
        area: &str,
        field: MeshGeometryField,
        rows: Option<Slice>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<NumericArray> {
        let g = self.mesh_geometry(area, metadata_max_bytes)?;
        self.read_mesh_geometry_cached(&g, field, rows, max_bytes)
    }
    /// Reuse a prior descriptor from this immutable session source.
    pub fn read_mesh_geometry_cached(
        &self,
        g: &MeshGeometry,
        field: MeshGeometryField,
        rows: Option<Slice>,
        max_bytes: usize,
    ) -> Result<NumericArray> {
        if g.mesh.id.source != self.path() {
            return Err(invalid("mesh descriptor belongs to another source"));
        }
        let d = g
            .fields
            .iter()
            .find(|d| d.field == field)
            .ok_or_else(|| invalid("unknown mesh field"))?;
        match &d.capability {
            Capability::Supported => selected(self, &d.path, rows, max_bytes),
            Capability::Unavailable { reason } => Err(Error::new(ErrorKind::NotFound, reason)),
            Capability::Unsupported { code, reason } => Err(Error::unsupported(*code, reason)),
            _ => Err(corrupt(
                d.capability.reason().unwrap_or("invalid geometry array"),
            )),
        }
    }
    pub fn mesh_geometry_tables(&self, max_bytes: usize) -> Result<MeshGeometryTables> {
        let attributes = exact_compound(self, &format!("{ROOT}/Attributes"), max_bytes)?;
        let path = format!("{ROOT}/Cell Info");
        let cell_info = if self.has_dataset(&path)? {
            Reported::supported(selected(self, &path, None, max_bytes)?)
        } else {
            Reported::unavailable("Cell Info absent")
        };
        Ok(MeshGeometryTables {
            attributes,
            cell_info,
        })
    }
}
