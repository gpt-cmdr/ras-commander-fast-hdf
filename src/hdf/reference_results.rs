//! Reference line and reference point results joined to their geometry features.
//!
//! The join is built only from the column-to-geometry links the read families already
//! prove (`FamilyDataset::geometry_locations` / `geometry_link`): each qualified numeric
//! result dataset is read once, exactly as stored, and every geometry feature lists the
//! (dataset, column) pairs linked to it. Nothing is matched by name here, and no column
//! order is assumed. A qualified dataset whose link is Unsupported or Corrupt refuses the
//! whole join with that status; absent geometry makes it Unavailable. Datasets the family
//! does not qualify are listed in `omitted` with their capability, never silently dropped.
use super::{
    families::{FamilyDataset, ReadFamily},
    lines::{GeometryFamily, GeometryFamilyData, GeometryFeature},
    reference_points::{ReferencePointFeature, ReferencePointGeometry},
    stored::StoredValues,
    NumericArray, ReadOnlyHdfSession,
};
use crate::{Capability, Error, ErrorKind, Reported, Result};

/// One qualified result dataset of the family, with its exact stored values.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReferenceResultDataset {
    /// The family descriptor (path, role, time group, Name axis, geometry identities).
    pub dataset: FamilyDataset,
    /// The whole stored array, `[time rows, columns]`, values not converted.
    pub values: NumericArray,
}

/// One column of `datasets[dataset].values` linked to a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReferenceSeriesLink {
    pub dataset: usize,
    pub column: u64,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReferenceLineResult {
    pub feature: GeometryFeature,
    /// In dataset order, then column order.
    pub series: Vec<ReferenceSeriesLink>,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReferencePointResult {
    pub feature: ReferencePointFeature,
    /// In dataset order, then column order.
    pub series: Vec<ReferenceSeriesLink>,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReferenceLineResults {
    pub geometry: GeometryFamilyData,
    pub datasets: Vec<ReferenceResultDataset>,
    /// One entry per geometry feature, in geometry row order (features without results
    /// have an empty `series`).
    pub features: Vec<ReferenceLineResult>,
    /// Family datasets not joined: (path, capability). Name axes are not listed.
    pub omitted: Vec<(String, Capability)>,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReferencePointResults {
    pub geometry: ReferencePointGeometry,
    pub datasets: Vec<ReferenceResultDataset>,
    pub features: Vec<ReferencePointResult>,
    pub omitted: Vec<(String, Capability)>,
}

fn corrupt(m: &str) -> Error {
    Error::new(ErrorKind::Corrupt, m)
}

/// The linked datasets with their values, the per-row links and the omitted datasets; or
/// the refusal every caller reports as the join's status.
struct Joined {
    datasets: Vec<ReferenceResultDataset>,
    links: Vec<Vec<ReferenceSeriesLink>>,
    omitted: Vec<(String, Capability)>,
}

impl ReadOnlyHdfSession {
    /// Reference-line result series joined to `Geometry/Reference Lines` features.
    /// `max_bytes` bounds each consulted dataset independently, as the family readers do.
    pub fn reference_line_results(
        &self,
        max_bytes: usize,
    ) -> Result<Reported<ReferenceLineResults>> {
        let geometry = match self.geometry_family(GeometryFamily::ReferenceLines, max_bytes) {
            Ok(g) => g,
            Err(e) => return Ok(refused(crate::capability::error_to_capability(e)?)),
        };
        if !geometry.is_supported() {
            return Ok(refused(geometry.capability().clone()));
        }
        let geometry = geometry
            .into_value()
            .expect("supported geometry has a value");
        let table = format!("{}/Attributes", GeometryFamily::ReferenceLines.group_path());
        let rows: Vec<u64> = geometry.features.iter().map(|f| f.id.row).collect();
        let joined = match self.join(ReadFamily::ReferenceLines, &table, &rows, max_bytes)? {
            Ok(j) => j,
            Err(cap) => return Ok(refused(cap)),
        };
        let features = geometry
            .features
            .iter()
            .cloned()
            .zip(joined.links)
            .map(|(feature, series)| ReferenceLineResult { feature, series })
            .collect();
        Ok(Reported::supported(ReferenceLineResults {
            geometry,
            datasets: joined.datasets,
            features,
            omitted: joined.omitted,
        }))
    }

    /// Reference-point result series joined to `Geometry/Reference Points` features.
    /// `max_bytes` bounds each consulted dataset independently.
    pub fn reference_point_results(
        &self,
        max_bytes: usize,
    ) -> Result<Reported<ReferencePointResults>> {
        let geometry = match self.reference_point_geometry(max_bytes) {
            Ok(g) => g,
            Err(e) => return Ok(refused(crate::capability::error_to_capability(e)?)),
        };
        if !geometry.is_supported() {
            return Ok(refused(geometry.capability().clone()));
        }
        let geometry = geometry
            .into_value()
            .expect("supported geometry has a value");
        let rows: Vec<u64> = geometry.features.iter().map(|f| f.id.row).collect();
        let joined = match self.join(
            ReadFamily::ReferencePoints,
            "Geometry/Reference Points/Attributes",
            &rows,
            max_bytes,
        )? {
            Ok(j) => j,
            Err(cap) => return Ok(refused(cap)),
        };
        let features = geometry
            .features
            .iter()
            .cloned()
            .zip(joined.links)
            .map(|(feature, series)| ReferencePointResult { feature, series })
            .collect();
        Ok(Reported::supported(ReferencePointResults {
            geometry,
            datasets: joined.datasets,
            features,
            omitted: joined.omitted,
        }))
    }

    /// Joins the family's qualified numeric datasets to geometry rows `rows` of `table`.
    /// `Ok(Err(capability))` is a refusal of the whole join; host errors propagate.
    fn join(
        &self,
        family: ReadFamily,
        table: &str,
        rows: &[u64],
        max_bytes: usize,
    ) -> Result<std::result::Result<Joined, Capability>> {
        let inventory = self.read_family_inventory(family, max_bytes)?;
        if !inventory.capability.is_supported() {
            return Ok(Err(inventory.capability));
        }
        let position: std::collections::BTreeMap<u64, usize> =
            rows.iter().enumerate().map(|(i, &r)| (r, i)).collect();
        if position.len() != rows.len() {
            return Err(corrupt("duplicate reference geometry row identities"));
        }
        let mut refusal: Option<Capability> = None;
        let mut linked: Vec<FamilyDataset> = Vec::new();
        let mut links = vec![Vec::new(); rows.len()];
        let mut omitted = Vec::new();
        for d in inventory.datasets {
            if d.role == "name_axis" {
                continue;
            }
            match &d.stored.capability {
                Capability::Supported => {}
                // Reference-point series prove their geometry identities while qualifying, so
                // an identity or corruption failure there is an unproven link, not a layout
                // the family merely does not qualify yet.
                Capability::Corrupt { .. } => return Ok(Err(d.stored.capability.clone())),
                Capability::Unsupported {
                    code: crate::UnsupportedCode::IdentityLayout,
                    ..
                } => {
                    refusal.get_or_insert_with(|| d.stored.capability.clone());
                    continue;
                }
                other => {
                    omitted.push((d.stored.path.clone(), other.clone()));
                    continue;
                }
            }
            if d.stored.storage != "numeric" {
                omitted.push((d.stored.path.clone(), d.stored.capability.clone()));
                continue;
            }
            match &d.geometry_link {
                Capability::Supported => {}
                Capability::Corrupt { .. } => return Ok(Err(d.geometry_link.clone())),
                // Unavailable (geometry absent, or a family layout with no link), Unsupported
                // and Delegated all mean the column identities are not proven here.
                other => {
                    refusal.get_or_insert_with(|| other.clone());
                    continue;
                }
            }
            let columns = *d
                .stored
                .shape
                .get(1)
                .ok_or_else(|| corrupt("linked reference result must be rank two"))?;
            if d.stored.shape.len() != 2 || d.geometry_locations.len() as u64 != columns {
                return Err(corrupt(
                    "reference result geometry identities do not align with its columns",
                ));
            }
            let index = linked.len();
            for (column, id) in d.geometry_locations.iter().enumerate() {
                let row = position
                    .get(&id.row)
                    .filter(|_| id.table == table && id.source == self.path())
                    .ok_or_else(|| corrupt("reference result links outside the geometry rows"))?;
                links[*row].push(ReferenceSeriesLink {
                    dataset: index,
                    column: column as u64,
                });
            }
            linked.push(d);
        }
        if let Some(cap) = refusal {
            return Ok(Err(cap));
        }
        // Values are read only once the whole join is proven.
        let mut datasets = Vec::with_capacity(linked.len());
        for d in linked {
            let values = match self.read_family_dataset(
                family,
                &d.stored.path,
                None,
                max_bytes,
                max_bytes,
            )? {
                StoredValues::Numeric(v) => v,
                _ => return Err(corrupt("qualified reference result is not numeric")),
            };
            if values.shape != d.stored.shape {
                return Err(corrupt(
                    "reference result read shape differs from its descriptor",
                ));
            }
            datasets.push(ReferenceResultDataset { dataset: d, values });
        }
        Ok(Ok(Joined {
            datasets,
            links,
            omitted,
        }))
    }
}

fn refused<T>(capability: Capability) -> Reported<T> {
    match capability {
        Capability::Unavailable { reason, .. } => Reported::unavailable(reason),
        Capability::Unsupported { code, reason, .. } => Reported::unsupported(code, reason),
        Capability::Corrupt { reason, .. } => Reported::corrupt(reason),
        other => Reported::unsupported(
            crate::UnsupportedCode::NotImplemented,
            format!("reference join refused: {other:?}"),
        ),
    }
}
