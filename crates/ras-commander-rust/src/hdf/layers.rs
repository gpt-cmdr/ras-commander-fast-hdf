//! Driver-neutral layer catalogue. Capabilities describe reads, not successful computation.
use super::{
    lines::GEOMETRY_FAMILIES, mesh_geometry::MeshGeometryField, Identity, NumericKind,
    ReadOnlyHdfSession,
};
use crate::{Capability, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LayerField {
    pub name: String,
    pub dataset: Option<String>,
    pub numeric_kind: Option<NumericKind>,
    pub row_count: Option<u64>,
    pub capability: Capability,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LayerDescriptor {
    /// Stable family spelling, with geometry/result namespaces kept distinct.
    pub mesh_result: Option<super::mesh_results::MeshResultVariable>,
    pub run_summary: Option<super::mesh_results::RunSummaryGroup>,
    pub family: String,
    pub area: Option<String>,
    pub name: String,
    pub identity: Option<Identity>,
    /// Stable domain spelling: cell, face, face_point, feature, series, sample, family.
    pub element_domain: String,
    /// point, line_string, polygon, topology, table or none. No repaired geometry implied.
    pub geometry_type: String,
    pub row_count: Option<u64>,
    pub fields: Vec<LayerField>,
    pub capability: Capability,
}
pub(super) fn layer(
    family: &str,
    name: &str,
    domain: &str,
    geometry: &str,
    capability: Capability,
) -> LayerDescriptor {
    LayerDescriptor {
        mesh_result: None,
        run_summary: None,
        family: family.into(),
        area: None,
        name: name.into(),
        identity: None,
        element_domain: domain.into(),
        geometry_type: geometry.into(),
        row_count: None,
        fields: Vec::new(),
        capability,
    }
}
fn domain(field: MeshGeometryField) -> (&'static str, &'static str) {
    use MeshGeometryField as F;
    match field {
        F::CellCenters => ("cell", "point"),
        F::CellManningsN
        | F::CellMinimumElevation
        | F::CellFaceInfo
        | F::CellFacePointIndexes
        | F::CellVolumeElevationInfo => ("cell", "table"),
        F::FacePointIsPerimeter => ("face_point", "table"),
        F::FaceMinimumElevation => ("face", "table"),
        F::FacePoints => ("face_point", "point"),
        F::FacePointIndexes | F::FaceCellIndexes => ("face", "topology"),
        F::FaceNormalsAndLength | F::FaceAreaElevationInfo | F::FacePerimeterInfo => {
            ("face", "table")
        }
        F::Perimeter => ("area", "polygon"),
        _ => ("sample", "table"),
    }
}
impl ReadOnlyHdfSession {
    /// Catalogue of qualified geometry arrays/families and existing result-family reports.
    /// Includes unavailable/unsupported families with their codes; no opaque GENERIC layer.
    /// max_bytes has the same per-dataset meaning as the underlying inventories/readers.
    pub fn layer_inventory(&self, max_bytes: usize) -> Result<Vec<LayerDescriptor>> {
        let mut out = Vec::new();
        let mesh_present = match self.has_group("Geometry/2D Flow Areas") {
            Ok(v) => v,
            Err(e) => {
                out.push(layer(
                    "mesh_geometry",
                    "mesh_inventory",
                    "family",
                    "none",
                    crate::capability::error_to_capability(e)?,
                ));
                false
            }
        };
        if mesh_present {
            let meshes = match self.mesh_inventory(max_bytes) {
                Ok(v) => v,
                Err(e) => {
                    out.push(layer(
                        "mesh_geometry",
                        "mesh_inventory",
                        "family",
                        "none",
                        crate::capability::error_to_capability(e)?,
                    ));
                    Vec::new()
                }
            };
            for m in meshes {
                let g = match self.mesh_geometry_from_descriptor(m.clone()) {
                    Ok(g) => g,
                    Err(e) => {
                        let mut l = layer(
                            "mesh_geometry",
                            &m.mesh_name,
                            "family",
                            "none",
                            crate::capability::error_to_capability(e)?,
                        );
                        l.area = Some(m.mesh_name.clone());
                        out.push(l);
                        continue;
                    }
                };
                for d in g.fields {
                    let (domain, geometry) = domain(d.field);
                    let mut l = layer(
                        "mesh_geometry",
                        d.field.as_str(),
                        domain,
                        geometry,
                        d.capability.clone(),
                    );
                    l.area = Some(m.mesh_name.clone());
                    l.identity = Some(m.id.clone());
                    l.row_count = if d.field == MeshGeometryField::Perimeter {
                        Some(1)
                    } else {
                        d.shape.first().copied()
                    };
                    l.fields.push(LayerField {
                        name: d.field.dataset_name().into(),
                        dataset: Some(d.path),
                        numeric_kind: d.kind,
                        row_count: d.shape.first().copied(),
                        capability: d.capability,
                    });
                    out.push(l);
                }
                for b in m.output_blocks {
                    let mut l = layer(
                        "mesh_results",
                        &format!("{}/{}", b.mode.as_str(), b.block_name),
                        "series",
                        "none",
                        b.results.capability,
                    );
                    l.area = Some(m.mesh_name.clone());
                    l.identity = Some(m.id.clone());
                    if b.results.variables.is_empty() && l.capability.is_supported() {
                        l.capability =
                            Capability::unavailable("output block has no readable variables");
                    }
                    for v in b.results.variables {
                        let probe = self.numeric_capability(&v.dataset);
                        let (numeric_kind, capability) = match probe {
                            Ok(c) => (c.kind, c.capability),
                            Err(e) => (None, crate::capability::error_to_capability(e)?),
                        };
                        l.fields.push(LayerField {
                            name: v.dataset.rsplit('/').next().unwrap_or(&v.dataset).into(),
                            dataset: Some(v.dataset.clone()),
                            numeric_kind,
                            row_count: v.shape.first().copied(),
                            capability,
                        });
                    }
                    out.push(l);
                }
            }
        }
        for &family in GEOMETRY_FAMILIES {
            let mut l = layer(
                "geometry",
                family.as_str(),
                "feature",
                if matches!(
                    family,
                    super::lines::GeometryFamily::RefinementRegions
                        | super::lines::GeometryFamily::CalibrationRegions
                ) {
                    "polygon"
                } else if family == super::lines::GeometryFamily::ReferencePoints {
                    "point"
                } else {
                    "line_string"
                },
                Capability::Supported,
            );
            if family == super::lines::GeometryFamily::ReferencePoints {
                match self.reference_point_geometry(max_bytes) {
                    Ok(v) => {
                        l.capability = v.capability().clone();
                        if let Some(g) = v.value() {
                            l.row_count = Some(g.features.len() as u64);
                            for c in &g.attributes.table.columns {
                                l.fields.push(LayerField {
                                    name: c.name.clone(),
                                    dataset: Some("Geometry/Reference Points/Attributes".into()),
                                    numeric_kind: None,
                                    row_count: None,
                                    capability: c.value.capability(),
                                });
                            }
                        }
                    }
                    Err(e) => l.capability = crate::capability::error_to_capability(e)?,
                }
                out.push(l);
                continue;
            }
            match self.geometry_family(family, max_bytes) {
                Ok(v) => {
                    l.capability = v.capability().clone();
                    if let Some(g) = v.value() {
                        l.row_count = Some(g.features.len() as u64);
                        for c in &g.attributes.table.columns {
                            l.fields.push(LayerField {
                                name: c.name.clone(),
                                dataset: Some(format!("{}/Attributes", family.group_path())),
                                numeric_kind: None,
                                row_count: None,
                                capability: c.value.capability(),
                            });
                        }
                    }
                }
                Err(e) => l.capability = crate::capability::error_to_capability(e)?,
            }
            out.push(l);
        }
        self.append_mesh_result_layers(&mut out, max_bytes)?;
        let xs_present = match self.has_dataset("Geometry/Cross Sections/Attributes") {
            Ok(v) => v,
            Err(e) => {
                out.push(layer(
                    "cross_section_geometry",
                    "cross_sections",
                    "family",
                    "none",
                    crate::capability::error_to_capability(e)?,
                ));
                false
            }
        };
        if xs_present {
            let mut l = layer(
                "cross_section_geometry",
                "cross_sections",
                "feature",
                "table",
                Capability::Supported,
            );
            match self.cross_sections(max_bytes) {
                Ok(g) => {
                    l.row_count = Some(g.sections.len() as u64);
                    for name in [
                        "river",
                        "reach",
                        "station",
                        "name",
                        "left_bank",
                        "right_bank",
                    ] {
                        l.fields.push(LayerField {
                            name: name.into(),
                            dataset: Some("Geometry/Cross Sections/Attributes".into()),
                            numeric_kind: None,
                            row_count: None,
                            capability: Capability::delegated(
                                "inspect per-section Reported fields",
                            ),
                        });
                    }
                }
                Err(e) => l.capability = crate::capability::error_to_capability(e)?,
            }
            out.push(l);
        }
        for (name, domain, geometry) in [
            ("Cell Points", "cell", "point"),
            ("Polygon Info", "area", "table"),
            ("Polygon Parts", "area", "table"),
            ("Polygon Points", "sample", "table"),
        ] {
            let path = format!("Geometry/2D Flow Areas/{name}");
            let mut l = layer(
                "mesh_root_geometry",
                name,
                domain,
                geometry,
                Capability::Supported,
            );
            let probe = (|| {
                if !self.has_dataset(&path)? {
                    return Ok(None);
                }
                let a = super::mesh_geometry::selected(self, &path, None, max_bytes)?;
                Ok(Some(a))
            })();
            match probe {
                Ok(Some(a)) => {
                    l.row_count = a.shape.first().copied();
                    l.fields.push(LayerField {
                        name: name.into(),
                        dataset: Some(path),
                        numeric_kind: self
                            .numeric_capability(&format!("Geometry/2D Flow Areas/{name}"))?
                            .kind,
                        row_count: l.row_count,
                        capability: Capability::Supported,
                    });
                }
                Ok(None) => {
                    l.capability = Capability::unavailable("canonical root geometry absent")
                }
                Err(e) => l.capability = crate::capability::error_to_capability(e)?,
            }
            out.push(l);
        }
        let polygon_tables: Vec<_> = out
            .iter()
            .filter(|l| l.family == "mesh_root_geometry" && l.name.starts_with("Polygon "))
            .collect();
        let capability = polygon_tables
            .iter()
            .find(|l| !l.capability.is_supported())
            .map(|l| l.capability.clone())
            .unwrap_or(Capability::Supported);
        let mut polygons = layer(
            "mesh_root_geometry",
            "area_polygons",
            "area",
            "polygon",
            capability,
        );
        polygons.row_count = polygon_tables
            .iter()
            .find(|l| l.name == "Polygon Info")
            .and_then(|l| l.row_count);
        polygons.fields = polygon_tables
            .iter()
            .flat_map(|l| l.fields.clone())
            .collect();
        out.push(polygons);
        // API 7: 1D geometry, results, summary and structure families.
        self.append_one_d_layers(&mut out, max_bytes)?;
        let caps = match self.plan_capabilities(max_bytes) {
            Ok(c) => c,
            Err(e) => {
                out.push(layer(
                    "results",
                    "plan_capabilities",
                    "family",
                    "none",
                    crate::capability::error_to_capability(e)?,
                ));
                return Ok(out);
            }
        };
        for f in caps.families {
            let mut l = layer(
                "results",
                f.family.as_str(),
                "family",
                "none",
                f.status.clone(),
            );
            for dataset in f.datasets {
                let probe = (|| {
                    if self.object_kind(&dataset)? == Some(false) {
                        Ok(Some(self.numeric_capability(&dataset)?))
                    } else {
                        Ok(None)
                    }
                })();
                let (numeric_kind, row_count, capability) = match probe {
                    Ok(Some(c)) => (c.kind, c.shape.first().copied(), c.capability),
                    Ok(None) => (None, None, f.status.clone()),
                    Err(e) => (None, None, crate::capability::error_to_capability(e)?),
                };
                l.fields.push(LayerField {
                    name: dataset.rsplit('/').next().unwrap_or(&dataset).into(),
                    dataset: Some(dataset),
                    numeric_kind,
                    row_count,
                    capability,
                });
            }
            out.push(l);
        }
        for f in caps.structures {
            if !f.status.is_supported() {
                out.push(layer(
                    "structure_results",
                    f.kind.as_str(),
                    "family",
                    "none",
                    f.status,
                ));
                continue;
            }
            let inventory = match self.structure_inventory(f.kind, max_bytes) {
                Ok(v) => v,
                Err(e) => {
                    out.push(layer(
                        "structure_results",
                        f.kind.as_str(),
                        "family",
                        "none",
                        crate::capability::error_to_capability(e)?,
                    ));
                    continue;
                }
            };
            for structure in inventory.structures {
                let mut l = layer(
                    "structure_results",
                    &structure.name,
                    "series",
                    "none",
                    Capability::Supported,
                );
                l.row_count = Some(inventory.time_rows);
                l.identity = Some(structure.identity.id);
                l.fields = structure
                    .series
                    .into_iter()
                    .map(|s| LayerField {
                        name: s.relative,
                        dataset: Some(s.path),
                        numeric_kind: s.kind,
                        row_count: s.shape.first().copied(),
                        capability: s.capability,
                    })
                    .collect();
                out.push(l);
            }
            if !inventory.family_series.is_empty() {
                let mut l = layer(
                    "structure_results",
                    f.kind.as_str(),
                    "series",
                    "none",
                    Capability::Supported,
                );
                l.row_count = Some(inventory.time_rows);
                l.fields = inventory
                    .family_series
                    .into_iter()
                    .map(|s| LayerField {
                        name: s.relative,
                        dataset: Some(s.path),
                        numeric_kind: s.kind,
                        row_count: s.shape.first().copied(),
                        capability: s.capability,
                    })
                    .collect();
                out.push(l);
            }
        }
        out.extend(self.read_family_layers(max_bytes)?);
        Ok(out)
    }
}
