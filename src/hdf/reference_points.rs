//! Stored reference-point geometry and qualified 7.0.1 Base/DSS series, and the exact
//! geometry identities of reference-line result columns.
use super::{
    encoding::map_error,
    families::FamilyDataset,
    mesh_geometry::{exact_compound, selected, ExactCompound},
    metadata::MetadataValue,
    stored::{self, StoredValues},
    Identity, NumericArray, NumericValues, ReadOnlyHdfSession,
};
use crate::{Error, ErrorKind, Reported, Result, UnsupportedCode};
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReferencePointFeature {
    pub id: Identity,
    pub name: String,
    pub area: String,
    /// Exact signed stored-cell index; no physical/ghost renumbering.
    pub cell_index: i32,
    /// Row in the stored Points array, aligned with Attributes.
    pub point_row: u64,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReferencePointGeometry {
    pub attributes: ExactCompound,
    pub points: NumericArray,
    pub features: Vec<ReferencePointFeature>,
}
fn corrupt(m: &str) -> Error {
    Error::new(ErrorKind::Corrupt, m)
}
fn identity(m: &str) -> Error {
    Error::unsupported(UnsupportedCode::IdentityLayout, m)
}
impl ReadOnlyHdfSession {
    /// Exact Attributes and Points, with one source identity per point. No synthetic Info/Parts.
    /// Each stored dataset and the feature allocation are bounded by max_bytes independently.
    pub fn reference_point_geometry(
        &self,
        max_bytes: usize,
    ) -> Result<Reported<ReferencePointGeometry>> {
        let root = "Geometry/Reference Points";
        // Canonical absence, then every dataset read below (see `presence`).
        if let Some(reason) = self.reference_point_prelude()? {
            return Ok(Reported::unavailable(reason));
        }
        let attr_path = format!("{root}/Attributes");
        let attributes = exact_compound(self, &attr_path, max_bytes)?;
        let strings = |name: &str| -> Result<Vec<String>> {
            match attributes
                .table
                .columns
                .iter()
                .find(|c| c.name == name)
                .map(|c| &c.value)
            {
                Some(MetadataValue::Strings(v)) => Ok(v.clone()),
                _ => Err(identity(
                    "reference-point Name/SA/2D fixed-text columns required",
                )),
            }
        };
        let names = strings("Name")?;
        let areas = strings("SA/2D")?;
        if !attributes.table.columns.iter().any(|c| {
            c.name == "Cell Index"
                && matches!(
                    c.attribute_type,
                    super::AttributeType::Int {
                        signed: true,
                        bytes: 4,
                        ..
                    }
                )
        }) {
            return Err(Error::unsupported(
                UnsupportedCode::Encoding,
                "reference-point Cell Index requires stored i32",
            ));
        }
        let Some(MetadataValue::Signed(cells)) = attributes
            .table
            .columns
            .iter()
            .find(|c| c.name == "Cell Index")
            .map(|c| &c.value)
        else {
            return Err(identity(
                "reference-point Cell Index signed column required",
            ));
        };
        if attributes.table.shape != [names.len() as u64]
            || areas.len() != names.len()
            || cells.len() != names.len()
        {
            return Err(corrupt("reference-point Attributes columns/shape disagree"));
        }
        stored::budget(
            names
                .len()
                .checked_mul(std::mem::size_of::<ReferencePointFeature>())
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidRequest,
                        "point identity allocation overflow",
                    )
                    .with_code(UnsupportedCode::ResourceLimit)
                })?,
            max_bytes,
        )?;
        let points = selected(self, &format!("{root}/Points"), None, max_bytes)?;
        if points.shape != [names.len() as u64, 2] {
            return Err(corrupt(
                "reference-point Points must align one XY row per Attributes row",
            ));
        }
        if !matches!(
            &points.values,
            NumericValues::F32(_) | NumericValues::F64(_)
        ) {
            return Err(Error::unsupported(
                UnsupportedCode::Encoding,
                "reference-point coordinates require stored float",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut features = Vec::new();
        for (row, ((name, area), &cell)) in names.into_iter().zip(areas).zip(cells).enumerate() {
            if name.is_empty() || area.is_empty() || cell < 0 {
                return Err(identity(
                    "only observed named 2D cell reference points are qualified",
                ));
            }
            if !seen.insert((name.clone(), area.clone())) {
                return Err(identity("ambiguous reference-point Name/area identity"));
            }
            if area.contains(['/', '\\']) {
                return Err(identity("reference-point area is not an exact group name"));
            }
            let path = format!("Geometry/2D Flow Areas/{area}/Cells Center Coordinate");
            if !self.has_dataset(&path)? {
                return Err(identity("reference-point area geometry absent"));
            }
            let shape = self
                .file
                .dataset(&path)
                .map_err(map_error)?
                .shape()
                .to_vec();
            if shape.len() != 2 || shape[1] != 2 || cell as u64 >= shape[0] {
                return Err(corrupt(
                    "reference-point stored-cell index outside area geometry",
                ));
            }
            let cell_index = i32::try_from(cell)
                .map_err(|_| identity("reference-point Cell Index exceeds observed i32 layout"))?;
            features.push(ReferencePointFeature {
                id: Identity::new(self.path(), &attr_path, row as u64),
                name,
                area,
                cell_index,
                point_row: row as u64,
            });
        }
        Ok(Reported::supported(ReferencePointGeometry {
            attributes,
            points,
            features,
        }))
    }
    pub(super) fn qualify_reference_point_series(
        &self,
        path: &str,
        d: &mut FamilyDataset,
        max: usize,
    ) -> Result<()> {
        let root = "Results/Unsteady/Output/Output Blocks/";
        let (block, tail) = path
            .strip_prefix(root)
            .and_then(|p| p.split_once("/Unsteady Time Series/"))
            .ok_or_else(|| {
                Error::unsupported(
                    UnsupportedCode::Schema,
                    "reference-point summaries/non-series layouts unqualified",
                )
            })?;
        if !matches!(block, "Base Output" | "DSS Hydrograph Output") {
            return Err(Error::unsupported(
                UnsupportedCode::Schema,
                "reference-point output block unqualified",
            ));
        }
        let (area, var) = if let Some(var) = tail.strip_prefix("Reference Points/") {
            (None, var)
        } else if let Some(rest) = tail.strip_prefix("2D Flow Areas/") {
            let (area, var) = rest
                .split_once("/Reference Points/")
                .ok_or_else(|| identity("reference-point area path"))?;
            if area.contains('/') {
                return Err(identity("reference-point area path"));
            }
            (Some(area), var)
        } else {
            return Err(identity("reference-point group path"));
        };
        if !matches!(var, "Name" | "Water Surface" | "Velocity" | "Direction")
            || (area.is_some() && !matches!(var, "Name" | "Water Surface"))
        {
            return Err(Error::unsupported(
                UnsupportedCode::Schema,
                "reference-point variable has no qualified fixture",
            ));
        }
        let parent = path
            .rsplit_once('/')
            .ok_or_else(|| corrupt("point series path"))?
            .0;
        let table = format!("{parent}/Name");
        let name_ds = stored::descriptor(self, &table, max)?;
        let StoredValues::Strings {
            shape,
            values: names,
        } = stored::read(self, &name_ds, None, max)?
        else {
            return Err(identity("point Name axis must be fixed text"));
        };
        if shape != [names.len() as u64] {
            return Err(corrupt("point Name axis must be rank one"));
        }
        let geometry = self.reference_point_geometry(max)?;
        let g = geometry
            .value()
            .ok_or_else(|| identity("point results require qualified point geometry"))?;
        let mut seen = std::collections::BTreeSet::new();
        let mut linked = Vec::new();
        let mut cells = Vec::new();
        let mut areas = Vec::new();
        for name in &names {
            if !seen.insert(name) {
                return Err(identity("duplicate point Name axis identity"));
            }
            let matches: Vec<_> = g
                .features
                .iter()
                .filter(|p| match area {
                    Some(area) => p.area == area && p.name == *name,
                    None => format!("{}|{}", p.name, p.area) == *name,
                })
                .collect();
            if matches.len() != 1 {
                return Err(identity(
                    "point Name axis has no unique exact geometry match",
                ));
            }
            linked.push(matches[0].id.clone());
            cells.push(i64::from(matches[0].cell_index));
            areas.push(matches[0].area.as_str());
        }
        stored::budget(
            names
                .len()
                .checked_mul(std::mem::size_of::<Identity>())
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidRequest,
                        "point identity allocation overflow",
                    )
                    .with_code(UnsupportedCode::ResourceLimit)
                })?,
            max,
        )?;
        d.domain = "reference_point".into();
        d.names = names;
        d.locations = (0..d.names.len())
            .map(|row| Identity::new(self.path(), &table, row as u64))
            .collect();
        d.geometry_locations = linked;
        d.geometry_link = crate::Capability::Supported;
        d.identity_table = Some(table);
        if var == "Name" {
            d.role = "name_axis".into();
            return Ok(());
        }
        if d.stored.storage != "numeric"
            || !matches!(
                d.stored.kind,
                Some(super::NumericKind::F32 | super::NumericKind::F64)
            )
        {
            return Err(Error::unsupported(
                UnsupportedCode::Encoding,
                "point series require stored float",
            ));
        }
        let group = format!("{root}{block}/Unsteady Time Series");
        let rows = self.family_time_rows(&group, max)?;
        if d.stored.shape != [rows, d.names.len() as u64] {
            return Err(corrupt(
                "point series shape disagrees with block Time/Name axes",
            ));
        }
        let attr = d
            .stored
            .attributes
            .iter()
            .find(|a| a.name == "Cells")
            .ok_or_else(|| identity("point result Cells identity attribute absent"))?;
        if attr.shape != [cells.len() as u64]
            || !matches!(&attr.value,MetadataValue::Signed(v) if *v==cells)
        {
            return Err(corrupt(
                "point result Cells differ from named geometry identities",
            ));
        }
        if area.is_none() {
            let attr = d
                .stored
                .attributes
                .iter()
                .find(|a| a.name == "2D Areas")
                .ok_or_else(|| identity("global point result area identities absent"))?;
            let table = exact_compound(self, "Geometry/2D Flow Areas/Attributes", max)?;
            let Some(MetadataValue::Strings(names)) = table
                .table
                .columns
                .iter()
                .find(|c| c.name == "Name")
                .map(|c| &c.value)
            else {
                return Err(identity("area Attributes Name axis unqualified"));
            };
            let MetadataValue::Signed(indices) = &attr.value else {
                return Err(identity("point area ordinals require signed integers"));
            };
            if attr.shape != [areas.len() as u64] || indices.len() != areas.len() {
                return Err(corrupt("point area identity shape"));
            }
            for (&index, area) in indices.iter().zip(areas) {
                if index < 0
                    || usize::try_from(index)
                        .ok()
                        .and_then(|i| names.get(i))
                        .map(String::as_str)
                        != Some(area)
                {
                    return Err(corrupt(
                        "point area ordinal differs from named geometry area",
                    ));
                }
            }
        }
        if var == "Direction" {
            if !d.stored.attributes.iter().any(|a| {
                a.name == "Units"
                    && matches!(&a.value,MetadataValue::Strings(v) if v.as_slice()==["deg"])
            }) {
                return Err(Error::unsupported(
                    UnsupportedCode::Schema,
                    "point Direction requires explicit deg units",
                ));
            }
            d.role = "direction_series".into();
        } else {
            d.role = "time_series".into();
        }
        d.time_group = Some(group);
        Ok(())
    }
    /// The column-to-geometry link of one reference-line result dataset, as
    /// (`geometry_locations`, `geometry_link`): Unavailable when the plan stores no
    /// reference-line geometry, Unsupported/Corrupt when the identities do not prove a link.
    /// Host errors propagate.
    pub(super) fn reference_line_link(
        &self,
        name_path: &str,
        names: &[String],
        attributes: &[super::metadata::TypedAttribute],
        max: usize,
    ) -> Result<(Vec<Identity>, crate::Capability)> {
        if !self.has_group("Geometry/Reference Lines")? {
            return Ok((
                Vec::new(),
                crate::Capability::unavailable("reference-line geometry absent"),
            ));
        }
        match self.reference_line_geometry_locations(name_path, names, attributes, max) {
            Ok(v) => Ok((v, crate::Capability::Supported)),
            Err(e) => Ok((Vec::new(), crate::capability::error_to_capability(e)?)),
        }
    }
    /// Geometry `Reference Lines/Attributes` identities of reference-line result columns.
    ///
    /// Each stored result `Name` entry must equal `<Name>|<SA-2D>` of exactly one geometry row
    /// with a valid name; any other layout (absent geometry, no fixed-text `SA-2D` column,
    /// duplicate or unmatched names) is `Unsupported(identity_layout)`. Two stored identities
    /// are cross-checked, and a disagreement is `Corrupt`: the `<name>|Faces` attribute of the
    /// `Name` axis must equal, in order, the geometry `Internal Faces` of the linked row (both
    /// required), and `2D Areas` ordinals, when the series stores them, must name the same
    /// areas through `2D Flow Areas/Attributes`. Column order is linked by name, never assumed.
    ///
    /// Per-area series (`<block>/Unsteady Time Series/2D Flow Areas/<area>/Reference Lines/*`,
    /// written by HEC-RAS 7.0.1) store bare `<name>` entries and no Faces attributes: the area
    /// is the path's, each entry must match exactly one geometry row with that Name and SA-2D,
    /// and the Faces cross-check uses the same block's `Reference Lines/Name` axis (required).
    fn reference_line_geometry_locations(
        &self,
        name_path: &str,
        names: &[String],
        attributes: &[super::metadata::TypedAttribute],
        max: usize,
    ) -> Result<Vec<Identity>> {
        // Every dataset of a reference-line block repeats the same Name-axis link; prove it
        // once per session. The key holds every input, and errors are never memoized.
        let (linked, linked_areas) = self.memo(
            format!("reference_line_rows:{:?}", (name_path, names, max)),
            || self.reference_line_rows(name_path, names, max),
        )?;
        let linked_areas: Vec<&str> = linked_areas.iter().map(String::as_str).collect();
        if let Some(attr) = attributes.iter().find(|a| a.name == "2D Areas") {
            let table = exact_compound(self, "Geometry/2D Flow Areas/Attributes", max)?;
            let Some(MetadataValue::Strings(area_names)) = table
                .table
                .columns
                .iter()
                .find(|c| c.name == "Name")
                .map(|c| &c.value)
            else {
                return Err(identity("area Attributes Name axis unqualified"));
            };
            let MetadataValue::Signed(indices) = &attr.value else {
                return Err(identity("line area ordinals require signed integers"));
            };
            if attr.shape != [linked_areas.len() as u64] || indices.len() != linked_areas.len() {
                return Err(corrupt("line area identity shape"));
            }
            for (&index, area) in indices.iter().zip(&linked_areas) {
                if index < 0
                    || usize::try_from(index)
                        .ok()
                        .and_then(|i| area_names.get(i))
                        .map(String::as_str)
                        != Some(*area)
                {
                    return Err(corrupt(
                        "line area ordinal differs from named geometry area",
                    ));
                }
            }
        }
        Ok(linked)
    }
    /// Geometry identities and `SA-2D` areas linked to a reference-line `Name` axis, with the
    /// `<name>|Faces` cross-check (see [`Self::reference_line_geometry_locations`]).
    fn reference_line_rows(
        &self,
        name_path: &str,
        names: &[String],
        max: usize,
    ) -> Result<(Vec<Identity>, Vec<String>)> {
        use super::lines::GeometryFamily;
        let geometry = self.geometry_family(GeometryFamily::ReferenceLines, max)?;
        let g = geometry
            .value()
            .filter(|_| geometry.is_supported())
            .ok_or_else(|| identity("line results require qualified reference-line geometry"))?;
        let Some(MetadataValue::Strings(areas)) = g
            .attributes
            .table
            .columns
            .iter()
            .find(|c| c.name == "SA-2D")
            .map(|c| &c.value)
        else {
            return Err(identity("reference-line SA-2D fixed-text column required"));
        };
        if areas.len() != g.features.len() {
            return Err(corrupt(
                "reference-line Attributes rows and features disagree",
            ));
        }
        let mut keys = std::collections::BTreeSet::new();
        for (f, area) in g.features.iter().zip(areas) {
            if !f.name_capability.is_supported() || f.name.is_empty() || area.is_empty() {
                return Err(identity("only named 2D reference lines are qualified"));
            }
            if !keys.insert(format!("{}|{}", f.name, area)) {
                return Err(identity("ambiguous reference-line Name/SA-2D identity"));
            }
        }
        stored::budget(
            names
                .len()
                .checked_mul(std::mem::size_of::<Identity>())
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidRequest,
                        "line identity allocation overflow",
                    )
                    .with_code(UnsupportedCode::ResourceLimit)
                })?,
            max,
        )?;
        // (area, block Name axis carrying the Faces attributes) for a per-area series.
        let per_area = match name_path.split_once("/Unsteady Time Series/2D Flow Areas/") {
            Some((block, rest)) => {
                let area = rest
                    .strip_suffix("/Reference Lines/Name")
                    .filter(|a| !a.is_empty() && !a.contains('/'))
                    .ok_or_else(|| identity("reference-line area path"))?;
                Some((
                    area,
                    format!("{block}/Unsteady Time Series/Reference Lines/Name"),
                ))
            }
            None => None,
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut linked = Vec::with_capacity(names.len());
        let mut linked_areas = Vec::with_capacity(names.len());
        let mut linked_rows = Vec::with_capacity(names.len());
        for name in names {
            if !seen.insert(name) {
                return Err(identity("duplicate line Name axis identity"));
            }
            let matches: Vec<usize> = g
                .features
                .iter()
                .zip(areas)
                .enumerate()
                .filter(|(_, (f, area))| match per_area {
                    Some((a, _)) => area.as_str() == a && f.name == *name,
                    None => format!("{}|{}", f.name, area) == *name,
                })
                .map(|(i, _)| i)
                .collect();
            let [i] = matches[..] else {
                return Err(identity(
                    "line Name axis has no unique exact geometry match",
                ));
            };
            linked.push(g.features[i].id.clone());
            linked_areas.push(areas[i].as_str());
            linked_rows.push(i);
        }
        let faces_path = per_area.as_ref().map_or(name_path, |(_, p)| p.as_str());
        let name_attributes =
            stored::attributes(self, faces_path, false, max).map_err(|e| match e.kind {
                ErrorKind::NotFound => {
                    identity("per-area line results require the block Name axis")
                }
                _ => e,
            })?;
        let internal = exact_compound(self, "Geometry/Reference Lines/Internal Faces", max)
            .map_err(|e| match e.kind {
                ErrorKind::NotFound => identity("reference-line Internal Faces required"),
                _ => e,
            })?;
        let signed = |name: &str| -> Result<&Vec<i64>> {
            match internal
                .table
                .columns
                .iter()
                .find(|c| c.name == name)
                .map(|c| &c.value)
            {
                Some(MetadataValue::Signed(v)) => Ok(v),
                _ => Err(identity(
                    "reference-line Internal Faces integer columns required",
                )),
            }
        };
        let (line_ids, face_ids) = (signed("Reference Line ID")?, signed("Face Index")?);
        if line_ids.len() != face_ids.len() {
            return Err(corrupt("reference-line Internal Faces columns disagree"));
        }
        for (row, id) in linked.iter().enumerate() {
            let key = format!("{}|Faces", g.features[linked_rows[row]].name);
            let stored = name_attributes
                .iter()
                .find(|a| a.name == key)
                .ok_or_else(|| identity("line Name axis Faces attribute absent"))?;
            let MetadataValue::Signed(faces) = &stored.value else {
                return Err(identity("line Name axis Faces require signed integers"));
            };
            let row = i64::try_from(id.row).map_err(|_| corrupt("line row overflow"))?;
            let geometry: Vec<i64> = line_ids
                .iter()
                .zip(face_ids)
                .filter(|(l, _)| **l == row)
                .map(|(_, f)| *f)
                .collect();
            if *faces != geometry {
                return Err(corrupt(
                    "line result Faces differ from geometry Internal Faces",
                ));
            }
        }
        Ok((
            linked,
            linked_areas.into_iter().map(str::to_owned).collect(),
        ))
    }
}
