//! Static 2D mesh corner (face-point) ground elevations.
//!
//! RASMapper's depth-weighted sloping interpolation uses one terrain elevation per mesh
//! corner (face point). HEC-RAS stores them in two places:
//!
//! - the geometry layout, written by 2D hydraulic-table preprocessing into a geometry or
//!   plan HDF: `Geometry/2D Flow Areas/<mesh>/FacePoints Elevation`;
//! - the mapping-cache layout, written by RASMapper into the result's `PostProcessing.hdf`:
//!   `Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas/<mesh>/
//!   Processed Data/Profile (Horizontal)/FacePoint Elevation` (singular `FacePoint`).
//!
//! These are terrain values, not results: they are distinct from corner water surfaces,
//! face water surfaces and node velocities. Both layouts were observed (HEC-RAS 7.0/7.0.1)
//! as rank-one little-endian `f4` arrays in `FacePoints Coordinate` row order and are read
//! exactly as stored. Nothing is interpolated, reordered or filled; NaN and sentinel values
//! are returned bit for bit. An array is qualified only when its length equals the mesh's
//! corner count. A cache file stores no coordinates, so its count must come from the plan or
//! geometry HDF it belongs to. This module never chooses between the two layouts and never
//! generates elevations: callers that hold both decide.
use super::{
    families::{FamilyDataset, FamilyInventory, ReadFamily},
    stored::{self, StoredDataset, StoredValues},
    ByteOrder, NumericKind, NumericValues, ReadOnlyHdfSession, Slice,
};
use crate::{
    capability::{absent_ok, error_to_capability},
    Capability, Error, ErrorKind, Reported, Result, UnsupportedCode,
};

/// Group holding one subgroup per 2D flow area in a geometry or plan HDF.
pub const GEOMETRY_ROOT: &str = "Geometry/2D Flow Areas";
/// Corner elevations of the geometry layout, relative to the mesh group.
pub const GEOMETRY_DATASET: &str = "FacePoints Elevation";
/// Corner coordinates; their row count is the mesh's corner count.
pub const CORNER_COORDINATES: &str = "FacePoints Coordinate";
/// Group holding one subgroup per 2D flow area in a `PostProcessing.hdf` mapping cache.
pub const CACHE_ROOT: &str =
    "Results/Unsteady/Output/Output Blocks/Base Output/Summary Output/2D Flow Areas";
/// Corner elevations of the mapping-cache layout, relative to the mesh group.
pub const CACHE_DATASET: &str = "Processed Data/Profile (Horizontal)/FacePoint Elevation";

/// Which stored layout a corner-elevation array comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CornerElevationLayout {
    /// `Geometry/2D Flow Areas/<mesh>/FacePoints Elevation` in a geometry or plan HDF.
    Geometry,
    /// `.../Processed Data/Profile (Horizontal)/FacePoint Elevation` in `PostProcessing.hdf`.
    MappingCache,
}
impl CornerElevationLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Geometry => "geometry",
            Self::MappingCache => "mapping_cache",
        }
    }
    /// The family role reported for datasets of this layout.
    pub fn role(self) -> &'static str {
        match self {
            Self::Geometry => "geometry_corner_elevation",
            Self::MappingCache => "mapping_cache_corner_elevation",
        }
    }
    /// Exact dataset path of this layout for one mesh. The mesh name must be one plain
    /// HDF path component.
    pub fn dataset_path(self, mesh: &str) -> Result<String> {
        if mesh.is_empty() || mesh == "." || mesh == ".." || mesh.contains(['/', '\\']) {
            return Err(invalid("mesh name must be one plain HDF path component"));
        }
        Ok(match self {
            Self::Geometry => format!("{GEOMETRY_ROOT}/{mesh}/{GEOMETRY_DATASET}"),
            Self::MappingCache => format!("{CACHE_ROOT}/{mesh}/{CACHE_DATASET}"),
        })
    }
}

/// How the corner count a corner-elevation array was checked against was established.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CornerCountSource {
    /// Rows of `Geometry/2D Flow Areas/<mesh>/FacePoints Coordinate` in the same file.
    Coordinates { path: String },
    /// A count supplied by the caller (for a cache, the corner count of the plan or
    /// geometry the cache belongs to).
    Caller,
}

/// One mesh's corner elevations, exactly as stored.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CornerElevations {
    pub mesh: String,
    pub layout: CornerElevationLayout,
    /// Source file identity, dataset path, stored shape and every dataset attribute.
    pub dataset: StoredDataset,
    /// The corner count the array length was validated against.
    pub corner_count: u64,
    pub corner_count_source: CornerCountSource,
    /// Stored little-endian `f4` values in `FacePoints Coordinate` row order.
    pub values: Vec<f32>,
}

fn corrupt(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt, m)
}
fn invalid(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidRequest, m)
}
fn unsupported(m: impl Into<String>) -> Error {
    Error::unsupported(UnsupportedCode::Schema, m)
}

/// `(mesh, layout)` for an exact corner-elevation dataset path of either layout.
fn split(path: &str) -> Option<(&str, CornerElevationLayout)> {
    if let Some(rest) = path
        .strip_prefix(GEOMETRY_ROOT)
        .and_then(|r| r.strip_prefix('/'))
    {
        if let Some(mesh) = rest.strip_suffix(GEOMETRY_DATASET) {
            if let Some(mesh) = mesh.strip_suffix('/') {
                return Some((mesh, CornerElevationLayout::Geometry));
            }
        }
    }
    if let Some(rest) = path
        .strip_prefix(CACHE_ROOT)
        .and_then(|r| r.strip_prefix('/'))
    {
        if let Some(mesh) = rest.strip_suffix(CACHE_DATASET) {
            if let Some(mesh) = mesh.strip_suffix('/') {
                return Some((mesh, CornerElevationLayout::MappingCache));
            }
        }
    }
    None
}

fn plain(mesh: &str) -> bool {
    !mesh.is_empty() && mesh != "." && mesh != ".." && !mesh.contains(['/', '\\'])
}

fn dataset(stored: StoredDataset, layout: CornerElevationLayout) -> FamilyDataset {
    FamilyDataset {
        stored,
        role: layout.role().into(),
        domain: "mesh_corner".into(),
        time_group: None,
        identity_table: None,
        locations: Vec::new(),
        names: Vec::new(),
        geometry_locations: Vec::new(),
        geometry_link: Capability::unavailable("no geometry link for this family"),
        stored_location_ids: None,
        time_of_extreme_row: None,
    }
}

impl ReadOnlyHdfSession {
    /// Mesh group names under `root` whose group holds `relative` as a dataset, sorted.
    fn corner_meshes(&self, root: &str, relative: &str) -> Result<Vec<String>> {
        let Some(members) = absent_ok(self.group_members(root))? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for mesh in members.groups {
            if plain(&mesh) && self.has_dataset(&format!("{root}/{mesh}/{relative}"))? {
                out.push(mesh);
            }
        }
        out.sort();
        Ok(out)
    }

    /// Rows of the mesh's `FacePoints Coordinate` in this file, or `None` when it is absent.
    fn corner_coordinate_rows(&self, mesh: &str) -> Result<Option<(u64, String)>> {
        let path = format!("{GEOMETRY_ROOT}/{mesh}/{CORNER_COORDINATES}");
        if !self.has_dataset(&path)? {
            return Ok(None);
        }
        let ds = self
            .file
            .dataset(&path)
            .map_err(super::encoding::map_error)?;
        match ds.shape() {
            [rows, 2] => Ok(Some((*rows, path))),
            _ => Err(corrupt(format!(
                "{CORNER_COORDINATES} of mesh {mesh:?} must have shape (corners, 2)"
            ))),
        }
    }

    /// Qualify one corner-elevation dataset: rank one, little-endian `f4`, and exactly one
    /// value per corner. The corner count comes from the same file's `FacePoints Coordinate`
    /// when present; `expected` (a caller's count) must then agree with it. A cache file
    /// without coordinates needs `expected`.
    fn corner_qualify(
        &self,
        path: &str,
        expected: Option<u64>,
        max: usize,
    ) -> Result<(StoredDataset, u64, CornerCountSource)> {
        let (mesh, layout) = split(path)
            .filter(|(m, _)| plain(m))
            .ok_or_else(|| invalid("not a mesh corner-elevation dataset path"))?;
        let d = stored::descriptor(self, path, max)?;
        if !d.capability.is_supported() {
            return Err(match d.capability {
                Capability::Corrupt { reason } => corrupt(reason),
                c => Error::unsupported(
                    c.code().unwrap_or(UnsupportedCode::Schema),
                    c.reason().unwrap_or("unqualified corner elevation storage"),
                ),
            });
        }
        let numeric = self.numeric_capability(path)?;
        if d.storage != "numeric"
            || numeric.kind != Some(NumericKind::F32)
            || numeric.byte_order != Some(ByteOrder::Little)
        {
            return Err(Error::unsupported(
                UnsupportedCode::Encoding,
                "corner elevations must be stored as little-endian f4",
            ));
        }
        let [stored_rows] = d.shape[..] else {
            return Err(unsupported(
                "corner elevations must be a one-dimensional array",
            ));
        };
        let (count, source) = match self.corner_coordinate_rows(mesh)? {
            Some((rows, coordinates)) => {
                if let Some(e) = expected {
                    if e != rows {
                        return Err(corrupt(format!(
                            "mesh {mesh:?} has {rows} corners in {CORNER_COORDINATES}, \
                             not the expected {e}"
                        )));
                    }
                }
                (rows, CornerCountSource::Coordinates { path: coordinates })
            }
            None if layout == CornerElevationLayout::Geometry => {
                return Err(corrupt(format!(
                    "mesh {mesh:?} has {GEOMETRY_DATASET} but no {CORNER_COORDINATES}"
                )))
            }
            None => match expected {
                Some(e) => (e, CornerCountSource::Caller),
                None => {
                    return Err(Error::unsupported(
                        UnsupportedCode::IdentityLayout,
                        "the corner count is not stored in this file; \
                         pass the mesh's corner count from its plan or geometry HDF",
                    ))
                }
            },
        };
        if stored_rows != count {
            return Err(corrupt(format!(
                "mesh {mesh:?} stores {stored_rows} corner elevations for {count} corners"
            )));
        }
        Ok((d, count, source))
    }

    pub(super) fn corner_elevation_inventory(&self, max: usize) -> Result<FamilyInventory> {
        let mut out = FamilyInventory {
            family: ReadFamily::MeshCornerElevations,
            capability: Capability::Supported,
            datasets: Vec::new(),
            groups: Vec::new(),
        };
        for (root, relative, layout) in [
            (
                GEOMETRY_ROOT,
                GEOMETRY_DATASET,
                CornerElevationLayout::Geometry,
            ),
            (
                CACHE_ROOT,
                CACHE_DATASET,
                CornerElevationLayout::MappingCache,
            ),
        ] {
            for mesh in self.corner_meshes(root, relative)? {
                let path = layout.dataset_path(&mesh)?;
                let stored = match self.corner_qualify(&path, None, max) {
                    Ok((d, _, _)) => d,
                    Err(e) => {
                        let cap = error_to_capability(e)?;
                        let mut d = if cap.code() == Some(UnsupportedCode::ResourceLimit) {
                            stored::refused_descriptor(self, &path, cap.clone())?
                        } else {
                            stored::inventory_descriptor(self, &path, max)?
                        };
                        d.capability = cap;
                        d
                    }
                };
                let mut row = dataset(stored, layout);
                if layout == CornerElevationLayout::Geometry {
                    row.identity_table =
                        Some(format!("{GEOMETRY_ROOT}/{mesh}/{CORNER_COORDINATES}"));
                }
                out.datasets.push(row);
            }
        }
        if out.datasets.is_empty() {
            out.capability = Capability::unavailable("mesh corner elevations absent");
        }
        Ok(out)
    }

    pub(super) fn corner_elevation_read(
        &self,
        path: &str,
        slices: Option<&[Slice]>,
        metadata_max: usize,
        max: usize,
    ) -> Result<StoredValues> {
        if split(path).is_none_or(|(m, _)| !plain(m)) {
            return Err(invalid(format!(
                "expected {GEOMETRY_ROOT}/<mesh>/{GEOMETRY_DATASET} or {CACHE_ROOT}/<mesh>/{CACHE_DATASET}"
            )));
        }
        if !self.has_dataset(path)? {
            return Err(
                Error::new(ErrorKind::NotFound, "corner elevation dataset absent")
                    .with_subject(path),
            );
        }
        let (d, _, _) = self.corner_qualify(path, None, metadata_max)?;
        stored::read(self, &d, slices, max)
    }

    /// One mesh's corner elevations in one stored layout, exactly as stored.
    ///
    /// `expected_corners` is the mesh's corner count from its plan or geometry HDF. It is
    /// required for a cache file that does not store `FacePoints Coordinate` itself, and it
    /// must agree with the stored coordinates when they are present. An absent dataset
    /// (or mesh) is `Unavailable`; a refused layout is `Unsupported`; a length that differs
    /// from the corner count is `Corrupt`. The whole array is read within `max_bytes`.
    pub fn mesh_corner_elevations(
        &self,
        mesh: &str,
        layout: CornerElevationLayout,
        expected_corners: Option<u64>,
        max_bytes: usize,
    ) -> Result<Reported<CornerElevations>> {
        let path = layout.dataset_path(mesh)?;
        if !self.has_dataset(&path)? {
            return Ok(Reported::unavailable(format!(
                "{} corner elevations absent for mesh {mesh:?}",
                layout.as_str()
            )));
        }
        let (d, count, source) = match self.corner_qualify(&path, expected_corners, max_bytes) {
            Ok(v) => v,
            Err(e) => {
                return Ok(match error_to_capability(e)? {
                    Capability::Unsupported { code, reason } => Reported::unsupported(code, reason),
                    Capability::Corrupt { reason } => Reported::corrupt(reason),
                    c => Reported::unsupported(
                        UnsupportedCode::Schema,
                        c.reason().unwrap_or("unqualified corner elevations"),
                    ),
                })
            }
        };
        let StoredValues::Numeric(a) = stored::read(self, &d, None, max_bytes)? else {
            return Err(corrupt("corner elevations must be numeric"));
        };
        let NumericValues::F32(values) = a.values else {
            return Err(corrupt("corner elevations must decode as f32"));
        };
        Ok(Reported::supported(CornerElevations {
            mesh: mesh.to_owned(),
            layout,
            dataset: d,
            corner_count: count,
            corner_count_source: source,
            values,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_round_trip_and_reject_unplain_meshes() {
        for layout in [
            CornerElevationLayout::Geometry,
            CornerElevationLayout::MappingCache,
        ] {
            let p = layout.dataset_path("Perimeter 1").unwrap();
            assert_eq!(split(&p), Some(("Perimeter 1", layout)));
            for bad in ["", ".", "..", "a/b", "a\\b"] {
                assert!(layout.dataset_path(bad).is_err());
            }
        }
        // The singular cache spelling is not the geometry spelling and vice versa.
        assert_eq!(split("Geometry/2D Flow Areas/m/FacePoint Elevation"), None);
        assert_eq!(
            split(&format!(
                "{CACHE_ROOT}/m/Processed Data/Profile (Horizontal)/FacePoints Elevation"
            )),
            None
        );
        assert_eq!(split("Geometry/2D Flow Areas/FacePoints Elevation"), None);
    }
}
