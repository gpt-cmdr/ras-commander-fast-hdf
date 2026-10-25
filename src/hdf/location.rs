//! Explicit stored Location mappings. Variable spelling and matching counts never choose a domain,
//! except the API 9 absent-Location exact-name table for six canonical Base Output 2D series.
use super::{
    mesh_results::{DomainRule, ElementDomain},
    metadata::{MetadataValue, TypedAttribute},
};
use crate::{Error, Result, UnsupportedCode};
fn missing(reason: impl Into<String>) -> Error {
    Error::unsupported(UnsupportedCode::IdentityLayout, reason)
}
fn value(attributes: &[TypedAttribute]) -> Result<&str> {
    let a = attributes
        .iter()
        .find(|a| a.name == "Location")
        .ok_or_else(|| missing("stored Location attribute absent"))?;
    match &a.value {
        MetadataValue::Strings(v) if v.len() == 1 && (a.shape.is_empty() || a.shape == [1]) => {
            Ok(v[0].as_str())
        }
        _ => Err(missing("stored Location requires one qualified text value")),
    }
}
pub(super) fn pipe(attributes: &[TypedAttribute]) -> Result<(&'static str, &'static str)> {
    match value(attributes)? {
        "Cell Water Surface" => Ok(("cell", "Cell Polygons Info")),
        "Face Flow" | "Face Velocity" | "Face Water Surface" => Ok(("face", "Face Polylines Info")),
        "Nodes" => Ok(("node", "Node Indices")),
        "Pipe Links" => Ok(("conduit", "Conduit Indices")),
        other => Err(missing(format!("unqualified pipe Location: {other}"))),
    }
}
pub(super) fn mesh(attributes: &[TypedAttribute]) -> Result<ElementDomain> {
    match value(attributes)? {
        // Singular Cell is observed on Davis/Glaise cumulative precipitation/infiltration.
        "Cell" | "Cells" => Ok(ElementDomain::Cell),
        "Faces" => Ok(ElementDomain::Face),
        other => Err(missing(format!("unqualified mesh Location: {other}"))),
    }
}
/// HEC-RAS 5.0.7 stable-core names observed stored without Location in 5.0.7 plan output,
/// compatible with 6.x/7.x output. Exact spelling only: no prefix, substring or case folding.
/// Mirrors ras-commander `_LEGACY_MESH_SERIES_DOMAINS` (PR #565).
const LEGACY_MESH_SERIES: [(&str, ElementDomain); 6] = [
    ("Depth", ElementDomain::Cell),
    ("Water Surface", ElementDomain::Cell),
    ("Face Velocity", ElementDomain::Face),
    ("Face Shear Stress", ElementDomain::Face),
    ("Face Area", ElementDomain::Face),
    ("Face Manning's n", ElementDomain::Face),
];
/// A present Location is authoritative (an invalid one refuses, never falling back). Only an
/// absent Location on a rank-two numeric series at the canonical Base Output path
/// (`legacy_eligible`) may use the exact-name table; the caller checks counts and Time rows.
pub(super) fn mesh_series(
    attributes: &[TypedAttribute],
    name: &str,
    legacy_eligible: bool,
) -> Result<(ElementDomain, DomainRule)> {
    if attributes.iter().any(|a| a.name == "Location") {
        return mesh(attributes).map(|d| (d, DomainRule::StoredLocation));
    }
    match LEGACY_MESH_SERIES.iter().find(|(n, _)| *n == name) {
        Some((_, d)) if legacy_eligible => Ok((*d, DomainRule::LegacyExactName)),
        _ => Err(missing("stored Location attribute absent")),
    }
}
