//! Layer-catalogue entries for the 1D families (API 7). Each family delegates to its
//! qualified reader; non-host failures stay per-layer capabilities, host errors propagate.
use super::layers::{layer, LayerDescriptor, LayerField};
use super::one_d_summary::OneDElementDomain;
use super::structures_1d::OneDGeometryGroup;
use super::xs_geometry::{OneDLineFamily, ONE_D_LINE_FAMILIES};
use super::xs_results::XsResultDomain;
use super::{Identity, ReadOnlyHdfSession, ResultMode};
use crate::{Capability, Error, ErrorKind, Result};

fn failure(e: Error) -> Result<Capability> {
    if e.kind == ErrorKind::InvalidRequest && e.code.is_none() {
        return Ok(Capability::corrupt(e.message));
    }
    crate::capability::error_to_capability(e)
}
/// An enumerated but empty family is Unavailable, never an empty Supported layer.
fn empty_status(c: Capability) -> Capability {
    if c.is_supported() {
        Capability::unavailable("family has no stored variables")
    } else {
        c
    }
}
fn field(
    name: &str,
    dataset: &str,
    kind: Option<super::NumericKind>,
    rows: Option<u64>,
    c: Capability,
) -> LayerField {
    LayerField {
        name: name.into(),
        dataset: Some(dataset.into()),
        numeric_kind: kind,
        row_count: rows,
        capability: c,
    }
}

impl ReadOnlyHdfSession {
    pub(super) fn append_one_d_layers(
        &self,
        out: &mut Vec<LayerDescriptor>,
        max_bytes: usize,
    ) -> Result<()> {
        // Cross-section geometry fields (absent XS table: covered by the existing XS layer).
        match self.xs_geometry_inventory(max_bytes) {
            Ok(inv) => {
                for d in inv.fields {
                    let mut l = layer(
                        "xs_geometry",
                        d.field.as_str(),
                        if d.records_are_sections {
                            "cross_section"
                        } else {
                            "feature"
                        },
                        "table",
                        d.capability.clone(),
                    );
                    l.row_count = d.info_shape.first().copied();
                    l.fields.push(field(
                        "info",
                        &d.info_path,
                        d.info_kind,
                        d.info_shape.first().copied(),
                        d.capability.clone(),
                    ));
                    l.fields.push(field(
                        "values",
                        &d.values_path,
                        d.values_kind,
                        d.values_shape.first().copied(),
                        d.capability,
                    ));
                    out.push(l);
                }
                if !inv.unqualified.is_empty() {
                    let mut l = layer(
                        "xs_geometry",
                        "unqualified",
                        "family",
                        "none",
                        Capability::unsupported(
                            crate::UnsupportedCode::NotImplemented,
                            "no qualified typed reader",
                        ),
                    );
                    for (p, c) in inv.unqualified {
                        l.fields.push(LayerField {
                            name: p.rsplit('/').next().unwrap_or(&p).into(),
                            dataset: Some(p),
                            numeric_kind: None,
                            row_count: None,
                            capability: c,
                        });
                    }
                    out.push(l);
                }
            }
            Err(e) if e.kind == ErrorKind::NotFound => {}
            Err(e) => out.push(layer(
                "xs_geometry",
                "inventory",
                "family",
                "none",
                failure(e)?,
            )),
        }
        for &family in ONE_D_LINE_FAMILIES {
            let geometry = if family == OneDLineFamily::StorageAreaPolygons {
                "polygon"
            } else {
                "line_string"
            };
            let mut l = layer(
                "one_d_lines",
                family.as_str(),
                "feature",
                geometry,
                Capability::Supported,
            );
            match self.one_d_line_family(family, max_bytes) {
                Ok(r) => {
                    l.capability = r.capability().clone();
                    if let Some(d) = r.value() {
                        l.row_count = Some(d.features.len() as u64);
                        l.identity = d
                            .features
                            .first()
                            .map(|f| Identity::new(self.path(), &f.id.table, 0));
                        for (n, a) in [
                            ("info", &d.info),
                            ("parts", &d.parts),
                            ("points", &d.points),
                        ] {
                            l.fields.push(LayerField {
                                name: n.into(),
                                dataset: Some(format!(
                                    "{}/{} {}",
                                    family.group_path(),
                                    family.prefix(),
                                    match n {
                                        "info" => "Info",
                                        "parts" => "Parts",
                                        _ => "Points",
                                    }
                                )),
                                numeric_kind: None,
                                row_count: a.shape.first().copied(),
                                capability: Capability::Supported,
                            });
                        }
                    }
                }
                Err(e) => l.capability = failure(e)?,
            }
            out.push(l);
        }
        for (name, geometry) in [
            ("river_reaches", "table"),
            ("river_junctions", "point"),
            ("xs_interpolation_surfaces", "table"),
        ] {
            let mut l = layer(
                "one_d_topology",
                name,
                "feature",
                geometry,
                Capability::Supported,
            );
            let r = match name {
                "river_reaches" => self
                    .river_reaches(max_bytes)
                    .map(|r| (r.capability().clone(), r.value().map(|v| v.len() as u64))),
                "river_junctions" => self.river_junctions(max_bytes).map(|r| {
                    (
                        r.capability().clone(),
                        r.value().map(|v| v.features.len() as u64),
                    )
                }),
                _ => self.xs_interpolation_surfaces(max_bytes).map(|r| {
                    (
                        r.capability().clone(),
                        r.value().map(|v| v.surfaces.len() as u64),
                    )
                }),
            };
            match r {
                Ok((c, n)) => {
                    l.capability = c;
                    l.row_count = n;
                }
                Err(e) => l.capability = failure(e)?,
            }
            out.push(l);
        }
        for mode in [ResultMode::Steady, ResultMode::Unsteady] {
            match self.xs_result_inventory(mode, max_bytes) {
                Ok(inv) => {
                    if inv.variables.is_empty() {
                        out.push(layer(
                            "xs_results",
                            &inv.group,
                            "family",
                            "none",
                            empty_status(inv.capability),
                        ));
                        continue;
                    }
                    for v in inv.variables {
                        let domain = match v.domain {
                            XsResultDomain::CrossSection
                            | XsResultDomain::CrossSectionComponents => "cross_section",
                            _ => "other",
                        };
                        let mut l = layer(
                            "xs_results",
                            &format!("{}/{}", mode.as_str(), v.name),
                            domain,
                            "none",
                            v.capability.clone(),
                        );
                        l.identity = Some(Identity::new(self.path(), &v.path, 0));
                        l.row_count = v.shape.first().copied();
                        l.fields.push(field(
                            &v.name,
                            &v.path,
                            v.kind,
                            v.shape.first().copied(),
                            v.capability,
                        ));
                        out.push(l);
                    }
                }
                Err(e) => out.push(layer(
                    "xs_results",
                    mode.as_str(),
                    "family",
                    "none",
                    failure(e)?,
                )),
            }
        }
        match self.one_d_summary_inventory(max_bytes) {
            Ok(inv) => {
                if inv.variables.is_empty() {
                    out.push(layer(
                        "one_d_summary",
                        &inv.group,
                        "family",
                        "none",
                        empty_status(inv.capability),
                    ));
                }
                for v in inv.variables {
                    let domain = match v.domain {
                        OneDElementDomain::CrossSection => "cross_section",
                        OneDElementDomain::StorageArea => "storage_area",
                        _ => "other",
                    };
                    let mut l = layer(
                        "one_d_summary",
                        &format!("{}/{}", v.family, v.name),
                        domain,
                        "none",
                        v.capability.clone(),
                    );
                    l.identity = Some(Identity::new(self.path(), &v.path, 0));
                    l.row_count = v.shape.first().copied();
                    l.fields.push(field(
                        &v.name,
                        &v.path,
                        v.kind,
                        v.shape.first().copied(),
                        v.capability,
                    ));
                    out.push(l);
                }
            }
            Err(e) => out.push(layer(
                "one_d_summary",
                "Summary Output",
                "family",
                "none",
                failure(e)?,
            )),
        }
        match self.one_d_structure_capabilities(max_bytes) {
            Ok(caps) => {
                for c in caps {
                    let label = format!("{}/{}", c.kind.as_str(), c.mode.as_str());
                    if !c.status.is_supported() {
                        out.push(layer(
                            "structure_1d_results",
                            &label,
                            "family",
                            "none",
                            c.status,
                        ));
                        continue;
                    }
                    match self.one_d_structure_inventory(c.kind, c.mode, max_bytes) {
                        Ok(inv) => {
                            for s in inv.structures {
                                let mut l = layer(
                                    "structure_1d_results",
                                    &s.name,
                                    "series",
                                    "none",
                                    Capability::Supported,
                                );
                                l.area = Some(label.clone());
                                l.row_count = Some(inv.rows);
                                l.identity = Some(s.identity.id);
                                l.fields = s
                                    .series
                                    .into_iter()
                                    .map(|x| LayerField {
                                        name: x.relative,
                                        dataset: Some(x.path),
                                        numeric_kind: x.kind,
                                        row_count: x.shape.first().copied(),
                                        capability: x.capability,
                                    })
                                    .collect();
                                out.push(l);
                            }
                            if !inv.family_series.is_empty() {
                                let mut l = layer(
                                    "structure_1d_results",
                                    &label,
                                    "series",
                                    "none",
                                    Capability::Supported,
                                );
                                l.row_count = Some(inv.rows);
                                l.fields = inv
                                    .family_series
                                    .into_iter()
                                    .map(|x| LayerField {
                                        name: x.relative,
                                        dataset: Some(x.path),
                                        numeric_kind: x.kind,
                                        row_count: x.shape.first().copied(),
                                        capability: x.capability,
                                    })
                                    .collect();
                                out.push(l);
                            }
                        }
                        Err(e) => out.push(layer(
                            "structure_1d_results",
                            &label,
                            "family",
                            "none",
                            failure(e)?,
                        )),
                    }
                }
            }
            Err(e) => out.push(layer(
                "structure_1d_results",
                "capabilities",
                "family",
                "none",
                failure(e)?,
            )),
        }
        let mut l = layer(
            "structure_1d_geometry",
            "structures",
            "feature",
            "table",
            Capability::Supported,
        );
        match self.structure_geometry(max_bytes) {
            Ok(r) => {
                l.capability = r.capability().clone();
                l.row_count = r.value().map(|g| g.records.len() as u64);
            }
            Err(e) => l.capability = failure(e)?,
        }
        out.push(l);
        for g in [
            OneDGeometryGroup::Structures,
            OneDGeometryGroup::PumpStations,
            OneDGeometryGroup::StorageAreas,
        ] {
            let mut l = layer(
                "one_d_geometry_group",
                g.as_str(),
                "family",
                "table",
                Capability::Supported,
            );
            match self.one_d_geometry_group(g, max_bytes) {
                Ok(r) => {
                    l.capability = r.capability().clone();
                    if let Some(d) = r.value() {
                        for (n, t) in &d.tables {
                            l.fields.push(field(
                                n,
                                &format!("{}/{n}", d.group),
                                None,
                                t.table.shape.first().copied(),
                                Capability::Supported,
                            ));
                        }
                        for (n, a) in &d.arrays {
                            l.fields.push(field(
                                n,
                                &format!("{}/{n}", d.group),
                                None,
                                a.shape.first().copied(),
                                Capability::Supported,
                            ));
                        }
                        for (n, c) in &d.unsupported {
                            l.fields.push(field(
                                n,
                                &format!("{}/{n}", d.group),
                                None,
                                None,
                                c.clone(),
                            ));
                        }
                    }
                }
                Err(e) => l.capability = failure(e)?,
            }
            out.push(l);
        }
        let mut l = layer(
            "structure_1d_geometry",
            "pump_stations",
            "feature",
            "point",
            Capability::Supported,
        );
        match self.pump_stations(max_bytes) {
            Ok(r) => {
                l.capability = r.capability().clone();
                l.row_count = r.value().map(|p| p.features.len() as u64);
            }
            Err(e) => l.capability = failure(e)?,
        }
        out.push(l);
        Ok(())
    }
}
