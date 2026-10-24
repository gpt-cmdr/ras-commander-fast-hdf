//! Catalogue integration delegates classification to the qualified mesh readers.
use super::{
    layers::{layer, LayerDescriptor},
    lines::GeometryFamily,
    metadata::MetadataValue,
    Identity, ReadOnlyHdfSession,
};
use crate::{Capability, Error, ErrorKind, Reported, Result, UnsupportedCode};
use std::collections::BTreeSet;
fn failure(e: Error) -> Result<Capability> {
    if e.kind == ErrorKind::InvalidRequest && e.code.is_none() {
        return Ok(Capability::corrupt(e.message));
    }
    crate::capability::error_to_capability(e)
}
fn reported_failure<T>(c: Capability) -> Reported<T> {
    match c {
        Capability::Unsupported { code, reason } => Reported::unsupported(code, reason),
        Capability::Unavailable { reason } => Reported::unavailable(reason),
        Capability::Corrupt { reason } => Reported::corrupt(reason),
        _ => Reported::corrupt("unexpected failure status"),
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BoundaryMatch {
    pub name: String,
    pub geometry: Identity,
    pub series_path: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BoundaryLinks {
    pub area: String,
    pub geometry_capability: Capability,
    pub series_capability: Capability,
    pub matched: Vec<BoundaryMatch>,
    pub unmatched_geometry: Vec<String>,
    pub unmatched_series: Vec<String>,
}
impl ReadOnlyHdfSession {
    pub(super) fn append_mesh_result_layers(
        &self,
        out: &mut Vec<LayerDescriptor>,
        max_bytes: usize,
    ) -> Result<()> {
        let areas = match self.mesh_names() {
            Ok(v) => v,
            Err(e) => {
                out.push(layer(
                    "mesh_result_catalogue",
                    "areas",
                    "family",
                    "none",
                    failure(e)?,
                ));
                vec![]
            }
        };
        for area in areas {
            for (family, inv) in [
                (
                    "mesh_summary",
                    self.mesh_summary_inventory(&area, max_bytes),
                ),
                ("mesh_series", self.mesh_series_inventory(&area, max_bytes)),
                (
                    "mesh_boundary_series",
                    self.mesh_boundary_inventory(&area, max_bytes),
                ),
            ] {
                match inv {
                    Ok(inv) => {
                        if inv.variables.is_empty() {
                            let mut l = layer(family, &inv.group, "family", "none", inv.capability);
                            l.area = Some(area.clone());
                            out.push(l);
                        }
                        for d in inv.variables {
                            let mut l = layer(
                                family,
                                &d.name,
                                d.domain.as_str(),
                                "none",
                                d.capability.clone(),
                            );
                            l.area = Some(area.clone());
                            l.identity = Some(Identity::new(self.path(), &d.path, 0));
                            l.row_count = d.shape.first().copied();
                            l.mesh_result = Some(d);
                            out.push(l);
                        }
                    }
                    Err(e) => {
                        let mut l = layer(family, &area, "family", "none", failure(e)?);
                        l.area = Some(area.clone());
                        out.push(l);
                    }
                }
            }
        }
        match self.run_summary(max_bytes) {
            Ok(r) => {
                if let Some(groups) = r.value() {
                    for g in groups {
                        let mut l = layer(
                            "run_summary",
                            &g.path,
                            "family",
                            "table",
                            r.capability().clone(),
                        );
                        l.run_summary = Some(g.clone());
                        out.push(l);
                    }
                } else {
                    out.push(layer(
                        "run_summary",
                        "Results/Unsteady/Summary",
                        "family",
                        "table",
                        r.capability().clone(),
                    ));
                }
            }
            Err(e) => out.push(layer(
                "run_summary",
                "Results/Unsteady/Summary",
                "family",
                "table",
                failure(e)?,
            )),
        }
        Ok(())
    }
    /// Exact stored Name and SA-2D association only. Unmatched sets are conditional on
    /// the independently reported side capabilities; absence never certifies completeness.
    /// Duplicate/invalid names or unproven SA-2D columns refuse linking, never choose a row.
    pub fn mesh_boundary_links(
        &self,
        area: &str,
        max_bytes: usize,
    ) -> Result<Reported<BoundaryLinks>> {
        // Validates caller area even when the geometry family is absent.
        let series = match self.mesh_boundary_inventory(area, max_bytes) {
            Ok(v) => v,
            Err(e)
                if e.kind == ErrorKind::NotFound
                    || e.kind == ErrorKind::InvalidRequest && e.code.is_none() =>
            {
                return Err(e)
            }
            Err(e) => return Ok(reported_failure(failure(e)?)),
        };
        let geom = match self.geometry_family(GeometryFamily::BoundaryConditions, max_bytes) {
            Ok(v) => v,
            Err(e) => return Ok(reported_failure(failure(e)?)),
        };
        let mut geometry = Vec::new();
        if let Some(g) = geom.value() {
            let col = g
                .attributes
                .table
                .columns
                .iter()
                .find(|c| c.name == "SA-2D");
            let Some(MetadataValue::Strings(areas)) = col.map(|c| &c.value) else {
                return Ok(Reported::unsupported(
                    UnsupportedCode::IdentityLayout,
                    "BC SA-2D association not proven",
                ));
            };
            if areas.len() != g.features.len() {
                return Ok(Reported::corrupt("BC area association row count mismatch"));
            }
            for (f, a) in g.features.iter().zip(areas) {
                if a == area {
                    if !f.name_capability.is_supported() {
                        return Ok(Reported::corrupt("invalid BC name"));
                    }
                    geometry.push(f);
                }
            }
        }
        let names = geometry
            .iter()
            .map(|f| f.name.clone())
            .collect::<BTreeSet<_>>();
        if names.len() != geometry.len() {
            return Ok(Reported::corrupt("duplicate BC name in area"));
        }
        let series_names = series
            .variables
            .iter()
            .map(|d| d.name.clone())
            .collect::<BTreeSet<_>>();
        let matched = geometry
            .iter()
            .filter(|f| series_names.contains(&f.name))
            .map(|f| BoundaryMatch {
                name: f.name.clone(),
                geometry: f.id.clone(),
                series_path: series
                    .variables
                    .iter()
                    .find(|d| d.name == f.name)
                    .expect("name from same list")
                    .path
                    .clone(),
            })
            .collect();
        Ok(Reported::supported(BoundaryLinks {
            area: area.into(),
            geometry_capability: geom.capability().clone(),
            series_capability: series.capability,
            matched,
            unmatched_geometry: names.difference(&series_names).cloned().collect(),
            unmatched_series: series_names.difference(&names).cloned().collect(),
        }))
    }
}
