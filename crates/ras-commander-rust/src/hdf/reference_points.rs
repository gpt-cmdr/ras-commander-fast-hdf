//! Stored reference-point geometry and qualified 7.0.1 Base/DSS series.
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
        if !self.has_group(root)? {
            return Ok(Reported::unavailable(
                "canonical reference-point geometry absent",
            ));
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
}
