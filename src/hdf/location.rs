//! Explicit stored Location mappings. Variable spelling and matching counts never choose a domain,
//! except the API 9 absent-Location exact-name table for six canonical Base Output 2D series and
//! the absent-Location summary rule ([`summary_shape_and_name`]), which needs both.
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
/// Location absent on a Summary Output array that no labelled rule qualifies: ras-commander's
/// h5py reader (`HdfResultsMesh._get_mesh_summary_values_output`) takes faces when the variable
/// name contains `Face` (exact case) and cells otherwise. The engine accepts that domain only
/// when the stored width proves it: equal to the named domain's stored count (ghosts included)
/// and different from the other domain's. Equal cell and face counts leave the width silent,
/// so the variable stays refused; so does a width that matches the other domain or neither.
/// The caller has already checked the rank and the `Time` label of a two-row array.
pub(super) fn summary_shape_and_name(
    name: &str,
    width: u64,
    counts: &[(ElementDomain, u64)],
) -> Result<(ElementDomain, DomainRule)> {
    let count = |d: ElementDomain| counts.iter().find(|(x, _)| *x == d).map(|(_, n)| *n);
    let (Some(cells), Some(faces)) = (count(ElementDomain::Cell), count(ElementDomain::Face))
    else {
        return Err(missing(
            "Location absent: cell and face counts are both needed to infer the domain",
        ));
    };
    if cells == faces {
        return Err(missing(format!(
            "Location absent: cell and face counts are equal ({cells}), so the width cannot confirm the domain the name implies"
        )));
    }
    let (domain, want) = if name.contains("Face") {
        (ElementDomain::Face, faces)
    } else {
        (ElementDomain::Cell, cells)
    };
    if width != want {
        return Err(missing(format!(
            "Location absent: width {width} differs from the {} count {want} the name implies",
            domain.as_str()
        )));
    }
    Ok((domain, DomainRule::SummaryShapeAndName))
}
