//! Classification, terrain, sediment and ancillary stored-read qualification.
use super::{
    encoding::map_error,
    families::{FamilyAttributes, FamilyDataset, FamilyInventory, ReadFamily},
    metadata::MetadataValue,
    stored::{self, StoredValues},
    Identity, NumericValues, ReadOnlyHdfSession, Slice,
};
use crate::{
    capability::error_to_capability, Capability, Error, ErrorKind, Result, UnsupportedCode,
};
use std::collections::BTreeSet;
pub(super) fn handles(f: ReadFamily) -> bool {
    matches!(
        f,
        ReadFamily::LandCover
            | ReadFamily::Infiltration
            | ReadFamily::Terrain
            | ReadFamily::Sediment
            | ReadFamily::Encroachments
            | ReadFamily::EventConditions
            | ReadFamily::SteadyOutput
    )
}
fn corrupt(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::Corrupt, m)
}
fn unsupported(m: impl Into<String>) -> Error {
    Error::unsupported(UnsupportedCode::Schema, m)
}
fn empty(family: ReadFamily) -> FamilyInventory {
    FamilyInventory {
        family,
        capability: Capability::Supported,
        datasets: Vec::new(),
        groups: Vec::new(),
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClassId {
    Signed(i64),
    Unsigned(u64),
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ClassificationLink {
    pub id: Identity,
    pub pixel_id: Option<ClassId>,
    pub name: String,
    pub variable_rows: Vec<Identity>,
    pub capability: Capability,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ClassificationCatalogue {
    pub layout: String,
    pub entries: Vec<ClassificationLink>,
    /// Variables with no matching raster-map name; never silently discarded.
    pub unmatched_variables: Vec<Identity>,
}
impl ReadOnlyHdfSession {
    /// Exact classification-name linking. Ambiguous duplicate names and unmatched
    /// classes remain explicit; no float conversion of pixel IDs or parameter defaulting.
    pub fn classification_catalogue(
        &self,
        family: ReadFamily,
        max: usize,
    ) -> Result<crate::Reported<ClassificationCatalogue>> {
        if !matches!(family, ReadFamily::LandCover | ReadFamily::Infiltration) {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "classification family must be land cover or infiltration",
            ));
        }
        let (paths, _) = self.environmental_paths(family, max)?;
        let mut out = ClassificationCatalogue {
            layout: String::new(),
            entries: Vec::new(),
            unmatched_variables: Vec::new(),
        };
        if paths.contains("IDs") && paths.contains("Names") && paths.contains("ManningsN") {
            out.layout = "legacy_parallel_arrays".into();
            let ds = stored::descriptor(self, "IDs", max)?;
            let ids = match stored::read(self, &ds, None, max)? {
                StoredValues::Numeric(a) => class_ids(a.values)?,
                StoredValues::Bitfield { raw_bytes, .. } => raw_bytes
                    .into_iter()
                    .map(|v| ClassId::Unsigned(v.into()))
                    .collect(),
                _ => {
                    return Err(corrupt(
                        "classification IDs require integer or byte bitfield storage",
                    ))
                }
            };
            stored::descriptor(self, "Names", max)?;
            let names = self.read_strings("Names")?;
            let n = self
                .file
                .dataset("ManningsN")
                .map_err(map_error)?
                .shape()
                .to_vec();
            if ds.shape != [ids.len() as u64] || names.len() != ids.len() || n != ds.shape {
                return Err(corrupt("classification parallel array lengths differ"));
            }
            for (row, (pixel_id, name)) in ids.into_iter().zip(names).enumerate() {
                out.entries.push(ClassificationLink {
                    id: Identity::new(self.path(), "IDs", row as u64),
                    pixel_id: Some(pixel_id),
                    name,
                    variable_rows: vec![Identity::new(self.path(), "ManningsN", row as u64)],
                    capability: Capability::Supported,
                });
            }
        } else if paths.contains("Variables") || paths.contains("Raster Map") {
            out.layout = "named_tables".into();
            let vars = if paths.contains("Variables") {
                Some(self.simple_compound("Variables", max)?)
            } else {
                None
            };
            let variable_names = vars
                .as_ref()
                .map(|t| table_names(t, "Name"))
                .transpose()?
                .unwrap_or_default();
            let (names, ids, table) = if paths.contains("Raster Map") {
                let t = self.simple_compound("Raster Map", max)?;
                let names = table_names(&t, "Name")?;
                let c = t
                    .columns
                    .iter()
                    .find(|c| c.name == "ID")
                    .ok_or_else(|| unsupported("Raster Map ID absent"))?;
                let ids = match &c.value {
                    MetadataValue::Signed(v) => v.iter().copied().map(ClassId::Signed).collect(),
                    MetadataValue::Unsigned(v) => {
                        v.iter().copied().map(ClassId::Unsigned).collect()
                    }
                    _ => return Err(unsupported("Raster Map ID must be proven integer")),
                };
                (names, Some(ids), "Raster Map")
            } else {
                (variable_names.clone(), None, "Variables")
            };
            let mut matched = BTreeSet::new();
            for (row, name) in names.into_iter().enumerate() {
                let indices: Vec<_> = variable_names
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| **v == name)
                    .map(|(i, _)| i)
                    .collect();
                matched.extend(indices.iter().copied());
                let capability = match indices.len() {
                    0 => Capability::unavailable("classification has no Variables match"),
                    1 => Capability::Supported,
                    _ => Capability::unsupported(
                        UnsupportedCode::IdentityLayout,
                        "ambiguous duplicate Variables name",
                    ),
                };
                out.entries.push(ClassificationLink {
                    id: Identity::new(self.path(), table, row as u64),
                    pixel_id: ids.as_ref().map(|v: &Vec<ClassId>| v[row].clone()),
                    name,
                    variable_rows: indices
                        .into_iter()
                        .map(|i| Identity::new(self.path(), "Variables", i as u64))
                        .collect(),
                    capability,
                });
            }
            out.unmatched_variables = (0..variable_names.len())
                .filter(|i| !matched.contains(i))
                .map(|i| Identity::new(self.path(), "Variables", i as u64))
                .collect();
        } else {
            return Ok(crate::Reported::unavailable(
                "no classification class table in this file",
            ));
        }
        Ok(crate::Reported::supported(out))
    }
    fn environmental_walk(
        &self,
        path: &str,
        depth: usize,
        datasets: &mut BTreeSet<String>,
        groups: &mut BTreeSet<String>,
    ) -> Result<()> {
        if depth > 24 || datasets.len() + groups.len() > 100_000 {
            return Err(Error::unsupported(
                UnsupportedCode::ResourceLimit,
                "environment hierarchy limit",
            ));
        }
        match self.object_kind(path)? {
            None => {}
            Some(false) => {
                datasets.insert(path.into());
            }
            Some(true) => {
                groups.insert(path.into());
                let members = self.group_members(path)?;
                for child in members.groups.into_iter().chain(members.datasets) {
                    self.environmental_walk(
                        &format!("{path}/{child}"),
                        depth + 1,
                        datasets,
                        groups,
                    )?;
                }
            }
        }
        Ok(())
    }
    fn environmental_paths(
        &self,
        family: ReadFamily,
        max: usize,
    ) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
        let mut paths = BTreeSet::new();
        let mut groups = BTreeSet::new();
        let attrs = stored::attributes(self, "/", true, max)?;
        let lc = attrs
            .iter()
            .find(|a| a.name == "LC Type")
            .and_then(|a| match &a.value {
                MetadataValue::Strings(v) => v.first().map(String::as_str),
                _ => None,
            });
        let flat = self.has_dataset("IDs")?
            && self.has_dataset("Names")?
            && self.has_dataset("ManningsN")?;
        let sidecar = match family {
            ReadFamily::LandCover => flat || matches!(lc, Some("LandCover" | "Soils")),
            ReadFamily::Infiltration => lc.is_some_and(|s| s.starts_with("Infiltration")),
            ReadFamily::Sediment => lc == Some("SedimentBedMaterial"),
            _ => false,
        };
        if sidecar {
            groups.insert("/".into());
            let m = self.group_members("/")?;
            for n in m.datasets.into_iter().chain(m.groups) {
                self.environmental_walk(&n, 0, &mut paths, &mut groups)?;
            }
        }
        let roots: &[&str] = match family {
            ReadFamily::LandCover => &["Geometry/Land Cover (Manning's n)"],
            ReadFamily::Infiltration => &["Geometry/Infiltration"],
            ReadFamily::Terrain => &["Terrain"],
            ReadFamily::Sediment => &[
                "Sediment",
                "Geometry/Sediment",
                "Plan Data/Sediment",
                "Event Conditions/Sediment",
                "Results/Sediment",
                "Results/Unsteady/Output/Output Blocks/Sediment",
                "Results/Unsteady/Output/Output Blocks/Sediment SE",
                "Results/Unsteady/Output/Output Blocks/Sediment Bed",
                "Results/Unsteady/Output/Output Blocks/Sediment Transport",
            ],
            ReadFamily::Encroachments => &[
                "Plan Data/Encroachments",
                "Plan Data/Encroachment Data",
                "Event Conditions/Steady Flow/Encroachment Data",
                "Plan Data/Plan Parameters/Encroachment",
                "Geometry/Cross Sections/Encroachment Stations",
                "Results/Steady/Output/Cross Sections/Encroachment Stations",
                "Results/Steady/Output/Output Blocks/Base Output/Steady Profiles",
            ],
            ReadFamily::EventConditions => &["Event Conditions"],
            ReadFamily::SteadyOutput => &["Results/Steady/Output"],
            _ => &[],
        };
        for root in roots {
            self.environmental_walk(root, 0, &mut paths, &mut groups)?;
        }
        if matches!(
            family,
            ReadFamily::LandCover | ReadFamily::Infiltration | ReadFamily::Sediment
        ) && self.has_group("Geometry/2D Flow Areas")?
        {
            for area in self.group_members("Geometry/2D Flow Areas")?.groups {
                let root = format!("Geometry/2D Flow Areas/{area}");
                for name in self
                    .group_members(&root)?
                    .datasets
                    .into_iter()
                    .chain(self.group_members(&root)?.groups)
                {
                    let take = match family {
                        ReadFamily::LandCover => name.contains("Manning") || name == "Land Cover",
                        ReadFamily::Infiltration => name.contains("Infiltration"),
                        ReadFamily::Sediment => name.contains("Sediment"),
                        _ => false,
                    };
                    if take {
                        self.environmental_walk(
                            &format!("{root}/{name}"),
                            0,
                            &mut paths,
                            &mut groups,
                        )?;
                    }
                }
            }
        }
        if family == ReadFamily::Sediment
            && (!paths.is_empty() || !groups.is_empty())
            && self.has_group("Geometry/2D Flow Areas")?
        {
            self.environmental_walk(
                "Geometry/2D Flow Areas/Attributes",
                0,
                &mut paths,
                &mut groups,
            )?;
            for area in self.group_members("Geometry/2D Flow Areas")?.groups {
                for name in ["Cells Center Coordinate", "Cells Surface Area"] {
                    self.environmental_walk(
                        &format!("Geometry/2D Flow Areas/{area}/{name}"),
                        0,
                        &mut paths,
                        &mut groups,
                    )?;
                }
            }
        }
        if family == ReadFamily::Encroachments {
            for root in [
                "Geometry/Cross Sections/Attributes",
                "Results/Steady/Output/Output Blocks/Base Output/Steady Profiles/Profile Names",
            ] {
                self.environmental_walk(root, 0, &mut paths, &mut groups)?;
            }
        }
        if !paths.is_empty() || !groups.is_empty() {
            groups.insert("/".into());
        }
        Ok((paths, groups))
    }
    pub(super) fn environmental_inventory(
        &self,
        family: ReadFamily,
        max: usize,
    ) -> Result<FamilyInventory> {
        let (paths, groups) = self.environmental_paths(family, max)?;
        let mut out = empty(family);
        for path in groups {
            out.groups.push(FamilyAttributes {
                id: Identity::new(self.path(), &path, 0),
                path: path.clone(),
                attributes: stored::attributes(self, &path, true, max)?,
            });
        }
        for path in paths {
            let mut d = FamilyDataset {
                stored: stored::inventory_descriptor(self, &path, max)?,
                role: "stored_table".into(),
                domain: "stored".into(),
                time_group: None,
                identity_table: None,
                locations: Vec::new(),
                names: Vec::new(),
                geometry_locations: Vec::new(),
                stored_location_ids: None,
                time_of_extreme_row: None,
            };
            if d.stored.capability.is_supported() {
                if let Err(e) = self.environmental_qualify(family, &mut d, max) {
                    d.stored.capability = error_to_capability(e)?;
                }
            }
            out.datasets.push(d);
        }
        if out.datasets.is_empty() && out.groups.is_empty() {
            out.capability = Capability::unavailable("canonical environmental family absent");
        }
        Ok(out)
    }
    pub(super) fn environmental_read(
        &self,
        family: ReadFamily,
        path: &str,
        slices: Option<&[Slice]>,
        metadata_max: usize,
        max: usize,
    ) -> Result<StoredValues> {
        if path.is_empty()
            || path
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == ".." || s.contains('\\'))
        {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "expected exact environmental dataset path",
            ));
        }
        let (paths, _) = self.environmental_paths(family, metadata_max)?;
        if !paths.contains(path) {
            return Err(Error::new(
                ErrorKind::NotFound,
                "dataset absent from selected environmental family",
            )
            .with_subject(path));
        }
        let mut d = FamilyDataset {
            stored: stored::descriptor(self, path, metadata_max)?,
            role: "stored_table".into(),
            domain: "stored".into(),
            time_group: None,
            identity_table: None,
            locations: Vec::new(),
            names: Vec::new(),
            geometry_locations: Vec::new(),
            stored_location_ids: None,
            time_of_extreme_row: None,
        };
        self.environmental_qualify(family, &mut d, metadata_max)?;
        stored::read(self, &d.stored, slices, max)
    }
    fn environmental_qualify(
        &self,
        family: ReadFamily,
        d: &mut FamilyDataset,
        max: usize,
    ) -> Result<()> {
        let path = d.stored.path.clone();
        let name = path
            .rsplit('/')
            .next()
            .ok_or_else(|| unsupported("empty environmental path"))?;
        if path.starts_with("Terrain/") {
            d.domain = "terrain".into();
            d.role = match name {
                "Mask" => "terrain_mask",
                "Min-Max" => "terrain_statistics",
                "Perimeter" => "terrain_perimeter",
                _ => "terrain_stitch",
            }
            .into();
            if !matches!(
                name,
                "Mask"
                    | "Min-Max"
                    | "Perimeter"
                    | "Stitch TIN Points"
                    | "Stitch TIN Triangles"
                    | "Stitches"
            ) {
                return Err(unsupported("unqualified embedded terrain dataset"));
            }
            if d.stored.shape.len() != 2 {
                return Err(corrupt("terrain arrays require rank two"));
            }
            return Ok(());
        }
        if family == ReadFamily::LandCover
            && !path.contains('/')
            && matches!(name, "IDs" | "Names" | "ManningsN")
            && self.has_dataset("IDs")?
            && self.has_dataset("Names")?
            && self.has_dataset("ManningsN")?
        {
            for key in ["IDs", "Names", "ManningsN"] {
                let ds = self.file.dataset(key).map_err(map_error)?;
                if ds.shape().len() != 1 || ds.shape() != d.stored.shape {
                    return Err(corrupt("legacy land-cover class arrays disagree"));
                }
            }
            d.domain = "classification".into();
            d.role = "class_axis".into();
            d.identity_table = Some("IDs".into());
            return Ok(());
        }
        if matches!(family, ReadFamily::LandCover | ReadFamily::Infiltration)
            && matches!(
                name,
                "Variables"
                    | "Raster Map"
                    | "Dependency Classifications"
                    | "Properties"
                    | "Calibration Table"
                    | "Base Overrides"
            )
        {
            if d.stored.storage != "compound" || d.stored.shape.len() != 1 {
                return Err(unsupported("unqualified classification table layout"));
            }
            d.domain = "classification".into();
        }
        // Polygon semantics are qualified only in observed parent namespaces.
        let parent = path
            .rsplit_once('/')
            .map(|(p, _)| p)
            .filter(|p| !p.is_empty());
        let polygon_parent = parent.filter(|p| {
            matches!(
                *p,
                "Classification Polygons"
                    | "Geometry/Land Cover (Manning's n)"
                    | "Geometry/Infiltration"
            )
        });
        if name.starts_with("Polygon ") {
            let root =
                polygon_parent.ok_or_else(|| unsupported("unqualified polygon namespace"))?;
            self.environmental_polygons(root, max)?;
            d.domain = "region".into();
            d.role = "polygon_array".into();
        } else if name == "Attributes" {
            if let Some(root) = polygon_parent {
                if self.has_dataset(&format!("{root}/Polygon Info"))? {
                    self.environmental_polygons(root, max)?;
                    d.domain = "region".into();
                    d.role = "polygon_array".into();
                }
            }
        }
        if let Some((area, var)) = path
            .strip_prefix("Geometry/2D Flow Areas/")
            .and_then(|s| s.split_once('/'))
        {
            let meshes = self.mesh_inventory(max)?;
            let mesh = meshes
                .iter()
                .find(|m| m.mesh_name == area)
                .ok_or_else(|| corrupt("preprocessed array mesh identity absent"))?;
            if d.stored.storage == "numeric" && d.stored.shape.len() == 1 {
                if var.contains("Face") {
                    let table = format!("Geometry/2D Flow Areas/{area}/Faces FacePoint Indexes");
                    let faces = self.file.dataset(&table).map_err(map_error)?;
                    if faces.shape().len() != 2 || faces.shape()[1] != 2 {
                        return Err(corrupt("preprocessed face identity shape"));
                    }
                    let n = faces.shape()[0];
                    if d.stored.shape[0] != n {
                        return Err(corrupt("preprocessed face count differs"));
                    }
                    d.domain = "face".into();
                    d.identity_table = Some(table);
                } else if d.stored.shape[0] == mesh.physical_cells
                    || d.stored.shape[0] == mesh.stored_cells
                {
                    d.domain = if d.stored.shape[0] == mesh.stored_cells {
                        "stored_cell"
                    } else {
                        "physical_cell"
                    }
                    .into();
                    d.identity_table = Some(format!(
                        "Geometry/2D Flow Areas/{area}/Cells Center Coordinate"
                    ));
                } else {
                    return Err(corrupt(
                        "preprocessed cell count differs from physical and stored counts",
                    ));
                }
                d.role = "preprocessed_values".into();
            }
        }
        let static_sediment = family == ReadFamily::Sediment
            && path.starts_with("Results/")
            && path.contains("/Sediment Bed/Unsteady Time Series/")
            && matches!(
                name,
                "Cell Initial Bed Elevation"
                    | "Subcell Area"
                    | "Subcell Initial Bed Elevation"
                    | "Subcell Initial Hydraulic Bed Elevations"
                    | "Subface Initial Bed Elevation"
                    | "Subface Length"
            );
        if static_sediment {
            if d.stored.shape.len() != 2 || d.stored.shape[0] != 1 {
                return Err(corrupt("static sediment property must have one stored row"));
            }
            d.role = "sediment_static".into();
        }
        if family == ReadFamily::Sediment
            && path.starts_with("Results/")
            && path.contains("/Unsteady Time Series/")
            && !matches!(
                name,
                "Time" | "Time Step" | "Time Date Stamp" | "Time Date Stamp (ms)"
            )
            && !static_sediment
            && d.stored.storage == "numeric"
        {
            let prefix = path
                .split_once("/Unsteady Time Series/")
                .ok_or_else(|| unsupported("missing time-series namespace"))?
                .0;
            let time = format!("{prefix}/Unsteady Time Series/Time");
            let axis = self.file.dataset(&time).map_err(map_error)?;
            if axis.shape().len() != 1 {
                return Err(corrupt("sediment Time axis rank"));
            }
            if d.stored.shape.len() < 2 || d.stored.shape[0] != axis.shape()[0] {
                return Err(unsupported(
                    "unqualified sediment result orientation or time alignment",
                ));
            }
            d.role = "sediment_series".into();
            d.time_group = Some(format!("{prefix}/Unsteady Time Series"));
        }
        if family == ReadFamily::Sediment
            && path.starts_with("Results/")
            && path.contains("/Sediment Time Series/")
        {
            d.role = if path.contains("/Cross Section SE/") {
                "sediment_station_elevation"
            } else {
                "sediment_series"
            }
            .into();
            if matches!(
                name,
                "Time" | "Time Step" | "Time Date Stamp" | "Time Date Stamp (ms)"
            ) || d.stored.storage != "numeric"
            {
                d.role = "stored_metadata".into();
            } else if !path.contains("/Cross Section SE/") {
                let prefix = path
                    .split_once("/Sediment Time Series/")
                    .ok_or_else(|| unsupported("missing sediment namespace"))?
                    .0;
                let time = format!("{prefix}/Sediment Time Series/Time");
                let axis = self.file.dataset(&time).map_err(map_error)?;
                if axis.shape().len() != 1 {
                    return Err(corrupt("sediment Time axis rank"));
                }
                if d.stored.shape.len() < 2 {
                    return Err(unsupported("unqualified dynamic sediment rank/orientation"));
                }
                if d.stored.shape[0] == 1 && axis.shape()[0] > 1 {
                    return Err(unsupported(
                        "unqualified one-row sediment result; Time alignment not proven",
                    ));
                }
                if d.stored.shape[0] != axis.shape()[0] {
                    return Err(corrupt("sediment time rows disagree"));
                }
                d.time_group = Some(format!("{prefix}/Sediment Time Series"));
            }
        }
        if family == ReadFamily::Encroachments {
            d.domain = "stored_metadata".into();
            d.role = "stored_metadata".into();
            if name == "Profile Names" {
                d.domain = "profile".into();
                d.role = "name_axis".into();
            } else if path.contains("/Cross Sections/") && d.stored.storage == "numeric" {
                let table = "Geometry/Cross Sections/Attributes";
                if self.has_dataset(table)? {
                    let xs = self.file.dataset(table).map_err(map_error)?;
                    let axis = if path.starts_with("Geometry/") {
                        d.stored.shape.first()
                    } else {
                        d.stored.shape.last()
                    };
                    if xs.shape().len() == 1 && axis == xs.shape().first() {
                        d.domain = "cross_section".into();
                        d.role = if name.contains("Encroachment") {
                            "encroachment_stations"
                        } else {
                            "floodway_output"
                        }
                        .into();
                        d.identity_table = Some(table.into());
                    }
                }
            }
        }
        Ok(())
    }
    fn environmental_polygons(&self, root: &str, max: usize) -> Result<()> {
        let info = stored::descriptor(self, &format!("{root}/Polygon Info"), max)?;
        let points = self
            .file
            .dataset(&format!("{root}/Polygon Points"))
            .map_err(map_error)?;
        if info.shape.len() != 2
            || info.shape[1] != 4
            || points.shape().len() != 2
            || points.shape()[1] != 2
        {
            return Err(corrupt("classification polygon shape"));
        }
        let attr = self
            .file
            .dataset(&format!("{root}/Attributes"))
            .map_err(map_error)?;
        if attr.shape() != [info.shape[0]] {
            return Err(corrupt("polygon Attributes/Info count mismatch"));
        }
        let StoredValues::Numeric(a) = stored::read(self, &info, None, max)? else {
            return Err(corrupt("polygon Info numeric required"));
        };
        let NumericValues::I32(iv) = a.values else {
            return Err(unsupported("unqualified polygon offsets"));
        };
        let part_path = format!("{root}/Polygon Parts");
        let pv = if self.has_dataset(&part_path)? {
            let d = stored::descriptor(self, &part_path, max)?;
            if d.shape.len() != 2 || d.shape[1] != 2 {
                return Err(corrupt("polygon Parts shape"));
            }
            let StoredValues::Numeric(a) = stored::read(self, &d, None, max)? else {
                return Err(corrupt("polygon Parts numeric required"));
            };
            let NumericValues::I32(v) = a.values else {
                return Err(unsupported("unqualified polygon parts"));
            };
            v
        } else {
            Vec::new()
        };
        let range = |start: i32, count: i32, bound: u64| -> Result<std::ops::Range<usize>> {
            let a = u64::try_from(start).map_err(|_| corrupt("negative polygon offset"))?;
            let n = u64::try_from(count).map_err(|_| corrupt("negative polygon count"))?;
            let end = a
                .checked_add(n)
                .filter(|&e| e <= bound)
                .ok_or_else(|| corrupt("polygon range outside stored array"))?;
            Ok(a as usize..end as usize)
        };
        for v in iv.as_chunks::<4>().0 {
            range(v[0], v[1], points.shape()[0])?;
            if !pv.is_empty() {
                let pr = range(v[2], v[3], (pv.len() / 2) as u64)?;
                for p in &pv.as_chunks::<2>().0[pr] {
                    range(
                        p[0],
                        p[1],
                        if root == "Classification Polygons" {
                            points.shape()[0]
                        } else {
                            v[1] as u64
                        },
                    )?;
                }
            } else if v[3] > 1 {
                return Err(corrupt("multipart polygon missing parts"));
            }
        }
        Ok(())
    }
}

fn table_names(t: &super::metadata::CompoundTable, name: &str) -> Result<Vec<String>> {
    let c = t
        .columns
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| unsupported("classification name column absent"))?;
    let MetadataValue::Strings(v) = &c.value else {
        return Err(unsupported("classification names require proven text"));
    };
    if t.shape != [v.len() as u64] {
        return Err(corrupt("classification name column count"));
    }
    Ok(v.clone())
}
fn class_ids(v: NumericValues) -> Result<Vec<ClassId>> {
    macro_rules! signed {
        ($v:expr) => {
            $v.into_iter().map(|v| ClassId::Signed(v.into())).collect()
        };
    }
    macro_rules! unsigned {
        ($v:expr) => {
            $v.into_iter()
                .map(|v| ClassId::Unsigned(v.into()))
                .collect()
        };
    }
    Ok(match v {
        NumericValues::I8(v) => signed!(v),
        NumericValues::I16(v) => signed!(v),
        NumericValues::I32(v) => signed!(v),
        NumericValues::I64(v) => signed!(v),
        NumericValues::U8(v) => unsigned!(v),
        NumericValues::U16(v) => unsigned!(v),
        NumericValues::U32(v) => unsigned!(v),
        NumericValues::U64(v) => unsigned!(v),
        _ => return Err(unsupported("classification IDs must be integer")),
    })
}
