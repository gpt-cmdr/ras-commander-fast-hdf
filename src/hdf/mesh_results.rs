//! Stored Base Output mesh results. No hydraulic value is recomputed by the readers.
use super::encoding::map_error;
use super::metadata::{typed_attribute, MetadataValue, TypedAttribute};
use super::{Identity, NumericArray, NumericKind, NumericValues, ReadOnlyHdfSession, Slice};
use crate::capability::absent_ok;
use crate::{Capability, Error, ErrorKind, Result, UnsupportedCode};

const BASE: &str = "Results/Unsteady/Output/Output Blocks/Base Output";
const GEOM: &str = "Geometry/2D Flow Areas";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ElementDomain {
    Cell,
    Face,
    Facepoint,
    Other,
}
impl ElementDomain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cell => "cell",
            Self::Face => "face",
            Self::Facepoint => "facepoint",
            Self::Other => "other",
        }
    }
}
/// How a variable's element domain was qualified, so callers can tell a stored-Location
/// domain from the absent-Location legacy exact-name rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DomainRule {
    /// Stored scalar `Location` (`Cells`/`Cell`/`Faces`).
    StoredLocation,
    /// Location absent; one of the six exact stable-core Base Output series names.
    LegacyExactName,
    /// Location absent; two-row summary labelled `WSEL`/`Time` with ft|m/days units.
    LabelledSummary,
    /// Boundary-condition columns are their own stored axis, not mesh elements.
    BoundaryColumns,
    /// No rule qualified a domain (the capability says why).
    Unqualified,
}
impl DomainRule {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StoredLocation => "stored_location",
            Self::LegacyExactName => "legacy_exact_name",
            Self::LabelledSummary => "labelled_summary",
            Self::BoundaryColumns => "boundary_columns",
            Self::Unqualified => "unqualified",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MeshResultVariable {
    pub name: String,
    pub path: String,
    pub domain: ElementDomain,
    /// The rule that produced `domain`; kept when the capability later refuses it.
    pub domain_rule: DomainRule,
    pub shape: Vec<u64>,
    pub kind: Option<NumericKind>,
    pub chunks: Option<Vec<u32>>,
    pub capability: Capability,
    /// All declared attributes, including units, row labels, BC columns and element IDs.
    pub attributes: Vec<TypedAttribute>,
    /// Only proven for two-row summaries explicitly labelling row 1 `Time`.
    pub time_of_extreme_row: Option<u64>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MeshResultInventory {
    pub group: String,
    pub capability: Capability,
    pub variables: Vec<MeshResultVariable>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MeshResultSelection {
    /// Dataset and row 0; individual element identities are `elements` in the same table.
    pub id: Identity,
    pub elements: Vec<u64>,
    pub rows: Slice,
    pub values: NumericArray,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MeshSeriesExtrema {
    pub id: Identity,
    pub elements: Vec<u64>,
    /// Row-major `[minimum, maximum]`, preserving stored F32/F64.
    pub values: NumericArray,
    /// Absolute stored time-row indices, same shape/order as values.
    pub time_indices: Vec<u64>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RunSummaryGroup {
    pub path: String,
    pub attributes: Vec<TypedAttribute>,
}
fn bad(m: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidRequest, m)
}
fn schema(m: impl Into<String>) -> Error {
    Error::unsupported(UnsupportedCode::Schema, m)
}
/// Resolve validated stored axes before constructing slices or indexing shape.
fn result_dimensions(d: &MeshResultVariable) -> Result<(u64, u64)> {
    match &d.capability {
        Capability::Supported => (),
        Capability::Unsupported { code, reason } => {
            return Err(Error::unsupported(*code, reason.clone()))
        }
        Capability::Corrupt { reason } => {
            return Err(Error::new(ErrorKind::Corrupt, reason.clone()))
        }
        _ => return Err(schema("unavailable result")),
    }
    match d.shape.as_slice() {
        [n] => Ok((1, *n)),
        [rows, n] => Ok((*rows, *n)),
        _ => Err(schema("unqualified result orientation")),
    }
}
fn budget(bytes: u64, max: usize) -> Result<()> {
    if bytes > max as u64 {
        Err(bad("mesh result byte budget exceeded").with_code(UnsupportedCode::ResourceLimit))
    } else {
        Ok(())
    }
}
fn component(s: &str) -> Result<()> {
    if s.is_empty() || s.contains('/') || s.contains('\\') || s == "." || s == ".." {
        Err(bad("expected an exact single HDF name"))
    } else {
        Ok(())
    }
}
fn width(k: NumericKind) -> u64 {
    match k {
        NumericKind::I8 | NumericKind::U8 => 1,
        NumericKind::I16 | NumericKind::U16 => 2,
        NumericKind::I32 | NumericKind::U32 | NumericKind::F32 => 4,
        _ => 8,
    }
}
fn row_count(s: Slice, n: u64) -> Result<u64> {
    if s.step == 0 || s.start > s.end || s.end > n {
        return Err(bad("invalid mesh result row range"));
    }
    Ok((s.end - s.start).div_ceil(s.step))
}
impl ReadOnlyHdfSession {
    /// Run and nested volume-accounting attributes, retaining raw datatype/payload evidence.
    /// Canonical absence is Unavailable; malformed present objects propagate as errors.
    pub fn run_summary(&self, max_bytes: usize) -> Result<crate::Reported<Vec<RunSummaryGroup>>> {
        self.memo(format!("run_summary:{:?}", (max_bytes,)), || {
            let root = "Results/Unsteady/Summary";
            if absent_ok(self.group_members(root))?.is_none() {
                return Ok(crate::Reported::unavailable("run summary absent"));
            }
            let mut pending = vec![(root.to_string(), 0)];
            let mut out = Vec::new();
            while let Some((path, depth)) = pending.pop() {
                if depth > 8 || out.len() > 100_000 {
                    return Err(Error::unsupported(
                        UnsupportedCode::ResourceLimit,
                        "run summary hierarchy limit",
                    ));
                }
                let group = self.file.group(&path).map_err(map_error)?;
                let attributes = group
                    .raw_attributes()
                    .map_err(map_error)?
                    .into_iter()
                    .map(|a| {
                        budget(a.raw_data.len() as u64, max_bytes)?;
                        typed_attribute(a)
                    })
                    .collect::<Result<Vec<_>>>()?;
                out.push(RunSummaryGroup {
                    path: path.clone(),
                    attributes,
                });
                let children = self.group_members(&path)?;
                // Only the volume-accounting subtree is part of this bounded API.
                for child in children.groups {
                    if path != root || child == "Volume Accounting" {
                        pending.push((format!("{path}/{child}"), depth + 1));
                    }
                }
            }
            Ok(crate::Reported::supported(out))
        })
    }
    /// Streaming min/max: first equal occurrence wins; first NaN propagates and its
    /// index wins, matching numpy argmin/argmax (not nanargmin/nanargmax). Values keep the
    /// bits at that index; numpy min/max can instead select a later signed zero.
    /// No dry/threshold masking. max_bytes bounds combined returned values+indices,
    /// and each temporary slab separately (chunk inflation/cache are storage costs).
    #[allow(clippy::too_many_arguments)]
    pub fn reduce_mesh_series(
        &self,
        area: &str,
        variable: &str,
        rows: Slice,
        elements: Option<&[u64]>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<MeshSeriesExtrema> {
        component(variable)?;
        let inv = self.mesh_series_inventory(area, metadata_max_bytes)?;
        let d = inv
            .variables
            .iter()
            .find(|v| v.name == variable)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "mesh series absent"))?;
        if d.shape.len() != 2 {
            return Err(schema("reduction requires rank two"));
        }
        if row_count(rows, d.shape[0])? == 0 {
            return Err(bad("empty reduction time window"));
        }
        let k = d.kind.ok_or_else(|| schema("unproven reduction kind"))?;
        if !matches!(k, NumericKind::F32 | NumericKind::F64) {
            return Err(Error::unsupported(
                UnsupportedCode::Encoding,
                "reduction requires stored f32 or f64",
            ));
        }
        let ne = elements.map_or(d.shape[1], |e| e.len() as u64);
        budget(
            ne.checked_mul(2 * (width(k) + 8))
                .ok_or_else(|| bad("extrema byte size overflow"))?,
            max_bytes,
        )?;
        let first = self.mr_read(
            d,
            Slice::new(rows.start, rows.start + 1, 1),
            elements,
            max_bytes,
        )?;
        let mut indices = vec![rows.start; ne as usize * 2];
        macro_rules! reduce {
            ($v:ident) => {{
                let NumericValues::$v(mut lo) = first.values.values else {
                    return Err(schema("reduction type changed"));
                };
                let mut hi = lo.clone();
                let mut at = rows.start.checked_add(rows.step);
                while let Some(row) = at.filter(|r| *r < rows.end) {
                    let slab = self.mr_read(
                        d,
                        Slice::new(row, row + 1, 1),
                        Some(&first.elements),
                        max_bytes,
                    )?;
                    let NumericValues::$v(vals) = slab.values.values else {
                        return Err(schema("reduction type changed"));
                    };
                    for (i, x) in vals.into_iter().enumerate() {
                        if !lo[i].is_nan() && (x.is_nan() || x < lo[i]) {
                            lo[i] = x;
                            indices[i] = row;
                        }
                        if !hi[i].is_nan() && (x.is_nan() || x > hi[i]) {
                            hi[i] = x;
                            indices[ne as usize + i] = row;
                        }
                    }
                    at = row.checked_add(rows.step);
                }
                lo.extend(hi);
                NumericValues::$v(lo)
            }};
        }
        let values = match k {
            NumericKind::F32 => reduce!(F32),
            NumericKind::F64 => reduce!(F64),
            _ => unreachable!(),
        };
        Ok(MeshSeriesExtrema {
            id: first.id,
            elements: first.elements,
            values: NumericArray {
                shape: vec![2, ne],
                values,
            },
            time_indices: indices,
        })
    }
    /// Inventory BC dataset names and exact Columns/Cells/Faces and unit attributes.
    pub fn mesh_boundary_inventory(
        &self,
        area: &str,
        max_bytes: usize,
    ) -> Result<MeshResultInventory> {
        self.mr_inventory(area, "Unsteady Time Series", true, max_bytes)
    }
    /// BC columns are stored column indices; any declared global IDs remain attributes.
    pub fn read_mesh_boundary_series(
        &self,
        area: &str,
        name: &str,
        rows: Slice,
        columns: Option<&[u64]>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<MeshResultSelection> {
        component(name)?;
        let d = self.mr_boundary_descriptor(area, name, metadata_max_bytes)?;
        self.mr_read(&d, rows, columns, max_bytes)
    }
    /// All directly stored cell/face series. Row count is cross-checked with Time.
    pub fn mesh_series_inventory(
        &self,
        area: &str,
        max_bytes: usize,
    ) -> Result<MeshResultInventory> {
        self.mr_inventory(area, "Unsteady Time Series", false, max_bytes)
    }
    /// Bounded `(time, element)` slab. Unsorted/duplicate elements retain caller order.
    pub fn read_mesh_series(
        &self,
        area: &str,
        variable: &str,
        rows: Slice,
        elements: Option<&[u64]>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<MeshResultSelection> {
        component(variable)?;
        let d = self.mr_descriptor(area, "Unsteady Time Series", variable, metadata_max_bytes)?;
        self.mr_read(&d, rows, elements, max_bytes)
    }

    #[allow(clippy::too_many_arguments)]
    fn mr_variable(
        &self,
        name: String,
        path: String,
        summary: bool,
        bc: bool,
        counts: &[(ElementDomain, u64)],
        times: Option<u64>,
        max_bytes: usize,
    ) -> Result<MeshResultVariable> {
        let cap = self.numeric_capability(&path)?;
        let attributes = self.mr_attributes(&path, max_bytes)?;
        let columns = match cap.shape.as_slice() {
            [n] if summary => Some(*n),
            [_, n] => Some(*n),
            _ => None,
        };
        // BC columns are their own stored column axis, not mesh elements.
        let row_labels = attributes.iter().find(|a| a.name == "Rows Variables");
        let row_units = attributes.iter().find(|a| a.name == "Units");
        let labelled_cell_summary = summary
            && !bc
            && cap.shape.len() == 2
            && cap.shape[0] == 2
            && path.starts_with(&format!("{BASE}/Summary Output/2D Flow Areas/"))
            && matches!(row_labels.map(|a| &a.value), Some(MetadataValue::Strings(v)) if v == &["WSEL", "Time"])
            && matches!(row_units.map(|a| &a.value), Some(MetadataValue::Strings(v)) if v.len()==2 && matches!(v[0].as_str(), "ft" | "m") && v[1]=="days");
        // The legacy exact-name rule is confined to rank-two numeric Base Output series at
        // their canonical path; summaries keep the labelled-summary rule above.
        let series_area = path
            .strip_prefix(&format!("{BASE}/Unsteady Time Series/2D Flow Areas/"))
            .and_then(|rest| rest.split_once('/'))
            .filter(|(area, n)| !area.is_empty() && *n == name)
            .map(|(area, _)| area.to_owned());
        let legacy_eligible = !summary
            && !bc
            && cap.capability.is_supported()
            && cap.shape.len() == 2
            && series_area.is_some();
        let location = if bc {
            Ok((ElementDomain::Other, DomainRule::BoundaryColumns))
        } else if labelled_cell_summary && !attributes.iter().any(|a| a.name == "Location") {
            Ok((ElementDomain::Cell, DomainRule::LabelledSummary))
        } else {
            super::location::mesh_series(&attributes, &name, legacy_eligible)
        };
        let (domain, domain_rule) = location
            .as_ref()
            .copied()
            .unwrap_or((ElementDomain::Other, DomainRule::Unqualified));
        let mut capability = cap.capability;
        if capability.is_supported() {
            capability = if columns.is_none() || (!summary && cap.shape.len() != 2) {
                Capability::unsupported(UnsupportedCode::Schema, "unqualified result orientation")
            } else if times.is_some_and(|t| cap.shape[0] != t) {
                Capability::corrupt("series rows differ from Time axis")
            } else if let Err(e) = location {
                crate::capability::error_to_capability(e)?
            } else if !bc {
                match counts.iter().find(|(d, _)| *d == domain) {
                    None => Capability::unsupported(
                        UnsupportedCode::IdentityLayout,
                        if domain_rule == DomainRule::LegacyExactName {
                            "geometry count for legacy exact-name domain absent"
                        } else {
                            "geometry count for stored Location absent"
                        },
                    ),
                    Some((_, n)) if Some(*n) != columns => {
                        Capability::corrupt(if domain_rule == DomainRule::LegacyExactName {
                            "legacy exact-name domain contradicts geometry count"
                        } else {
                            "known mesh Location contradicts geometry count"
                        })
                    }
                    Some((_, n))
                        if !summary
                            && series_area.is_some()
                            && domain == ElementDomain::Cell
                            && matches!(
                                domain_rule,
                                DomainRule::LegacyExactName | DomainRule::StoredLocation
                            ) =>
                    {
                        // PR #565 parity (`_mesh_series_id_dim`): on both series domain paths
                        // (absent-Location legacy and stored Location), a recorded physical
                        // Cell Count above the stored cell rows (ghosts included) refuses the
                        // cell domain. Summaries and BC columns keep their own rules.
                        match self.mr_physical_cell_count(
                            series_area.as_deref().unwrap_or_default(),
                            max_bytes,
                        ) {
                            Ok(Some(p)) if p > *n => Capability::corrupt(
                                "physical Cell Count exceeds stored cell geometry rows",
                            ),
                            Ok(_) => Capability::Supported,
                            Err(e) => crate::capability::error_to_capability(e)?,
                        }
                    }
                    Some(_) => Capability::Supported,
                }
            } else {
                Capability::Supported
            };
        }
        let time_of_extreme_row = if summary
            && cap.shape.first() == Some(&2)
            && cap.shape.len() == 2
            && attributes.iter().any(|a| {
                matches!(a.name.as_str(), "Rows Variables" | "Row Variables")
                    && matches!(&a.value,MetadataValue::Strings(v) if v.len()==2 && v[1]=="Time")
            }) {
            Some(1)
        } else {
            None
        };
        Ok(MeshResultVariable {
            name,
            path,
            domain,
            domain_rule,
            shape: cap.shape,
            kind: cap.kind,
            chunks: cap.chunks,
            capability,
            attributes,
            time_of_extreme_row,
        })
    }
    fn mr_descriptor(
        &self,
        area: &str,
        family: &str,
        name: &str,
        max_bytes: usize,
    ) -> Result<std::sync::Arc<MeshResultVariable>> {
        component(name)?;
        self.memo(
            format!("mr_descriptor:{:?}", (area, family, name, max_bytes)),
            || {
                let inv = self.mr_inventory(area, family, false, max_bytes)?;
                let d = inv
                    .variables
                    .into_iter()
                    .find(|v| v.name == name)
                    .ok_or_else(|| {
                        Error::new(
                            ErrorKind::NotFound,
                            if family == "Summary Output" {
                                "summary variable absent"
                            } else {
                                "mesh series absent"
                            },
                        )
                    })?;
                Ok(std::sync::Arc::new(d))
            },
        )
    }
    fn mr_boundary_descriptor(
        &self,
        area: &str,
        name: &str,
        max_bytes: usize,
    ) -> Result<std::sync::Arc<MeshResultVariable>> {
        component(area)?;
        component(name)?;
        self.memo(
            format!("mr_bc_descriptor:{:?}", (area, name, max_bytes)),
            || {
                if absent_ok(
                    self.file
                        .group(&format!("{GEOM}/{area}"))
                        .map_err(map_error),
                )?
                .is_none()
                {
                    return Err(Error::new(ErrorKind::NotFound, "unknown mesh area"));
                }
                let path = format!(
                    "{BASE}/Unsteady Time Series/2D Flow Areas/{area}/Boundary Conditions/{name}"
                );
                if absent_ok(self.file.dataset(&path).map_err(map_error))?.is_none() {
                    return Err(Error::new(ErrorKind::NotFound, "boundary series absent"));
                }
                let rows = self.memo(format!("mr_time_rows:{max_bytes}"), || {
                    Ok(self.unsteady_time_axis(max_bytes)?.rows)
                })?;
                Ok(std::sync::Arc::new(self.mr_variable(
                    name.into(),
                    path,
                    false,
                    true,
                    &[],
                    Some(rows),
                    max_bytes,
                )?))
            },
        )
    }
    /// Read requested BC lines once per session/metadata budget. None selects all.
    /// Output bytes plus element identities across the entire batch must fit max_bytes.
    pub fn read_mesh_boundary_batch(
        &self,
        area: &str,
        names: Option<&[&str]>,
        rows: Slice,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<Vec<(String, MeshResultSelection)>> {
        let names = match names {
            Some(n) => n.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            None => self
                .mesh_boundary_inventory(area, metadata_max_bytes)?
                .variables
                .into_iter()
                .map(|v| v.name)
                .collect(),
        };
        let mut remaining = max_bytes;
        let mut out = Vec::new();
        for name in names {
            let d = self.mr_boundary_descriptor(area, &name, metadata_max_bytes)?;
            let r = self.mr_read(&d, rows, None, remaining)?;
            let kind = d.kind.ok_or_else(|| schema("unproven numeric kind"))?;
            let used = r
                .values
                .shape
                .iter()
                .try_fold(1usize, |n, &v| n.checked_mul(usize::try_from(v).ok()?))
                .and_then(|n| n.checked_mul(width(kind) as usize));
            let used = used
                .and_then(|n| n.checked_add(r.elements.len().checked_mul(8)?))
                .ok_or_else(|| bad("batch byte overflow"))?;
            remaining = remaining
                .checked_sub(used)
                .ok_or_else(|| bad("batch byte overflow"))?;
            out.push((name, r));
        }
        Ok(out)
    }
    /// Values-only summary with identical validation and byte-budget verdicts.
    pub fn read_mesh_summary_values(
        &self,
        area: &str,
        variable: &str,
        elements: Option<&[u64]>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<NumericArray> {
        let d = self.mr_descriptor(area, "Summary Output", variable, metadata_max_bytes)?;
        let (rows, _) = result_dimensions(&d)?;
        self.mr_values(&d, Slice::new(0, rows, 1), elements, max_bytes)
    }

    fn mr_attributes(&self, path: &str, max_bytes: usize) -> Result<Vec<TypedAttribute>> {
        let ds = self.file.dataset(path).map_err(map_error)?;
        ds.raw_attributes()
            .iter()
            .map(|a| {
                budget(a.raw_data.len() as u64, max_bytes)?;
                typed_attribute(a.clone())
            })
            .collect()
    }
    /// Physical (non-ghost) cell count exactly as ras-commander PR #565 reads it
    /// (`HdfMesh._physical_cell_counts`): the `Geometry/2D Flow Areas/Attributes` row whose
    /// exact Name is `area` (last duplicate wins), only when Cell Count is finite and > 0,
    /// truncated like Python `int`. An absent table or Cell Count column records nothing.
    fn mr_physical_cell_count(&self, area: &str, max_bytes: usize) -> Result<Option<u64>> {
        self.memo(
            format!("mr_physical_cell_count:{:?}", (area, max_bytes)),
            || {
                let Some(table) =
                    absent_ok(self.simple_compound(&format!("{GEOM}/Attributes"), max_bytes))?
                else {
                    return Ok(None);
                };
                let Some(counts) = table.columns.iter().find(|c| c.name == "Cell Count") else {
                    return Ok(None);
                };
                let names = match table
                    .columns
                    .iter()
                    .find(|c| c.name == "Name")
                    .map(|c| &c.value)
                {
                    Some(MetadataValue::Strings(v)) => v,
                    _ => {
                        return Err(Error::unsupported(
                            UnsupportedCode::Encoding,
                            "mesh Attributes Name requires proven fixed strings",
                        ))
                    }
                };
                let counts: Vec<f64> = match &counts.value {
                    MetadataValue::Signed(v) => v.iter().map(|&x| x as f64).collect(),
                    MetadataValue::Unsigned(v) => v.iter().map(|&x| x as f64).collect(),
                    MetadataValue::Float(v) => v.clone(),
                    _ => {
                        return Err(Error::unsupported(
                            UnsupportedCode::Encoding,
                            "mesh Attributes Cell Count requires a proven numeric column",
                        ))
                    }
                };
                if names.len() != counts.len() {
                    return Err(Error::new(
                        ErrorKind::Corrupt,
                        "mesh attribute column lengths differ",
                    ));
                }
                Ok(names
                    .iter()
                    .zip(counts)
                    .filter(|(n, c)| n.as_str() == area && c.is_finite() && *c > 0.0)
                    .map(|(_, c)| c.trunc() as u64)
                    .next_back())
            },
        )
    }
    /// `None` when `.../Unsteady Time Series/Time` is a present, rank-one, proven numeric
    /// dataset; otherwise the refusal applied to every otherwise-qualified series.
    fn mr_time_dataset_refusal(&self) -> Result<Option<Capability>> {
        let path = format!("{BASE}/Unsteady Time Series/Time");
        let Some(time) = absent_ok(self.numeric_capability(&path))? else {
            return Ok(Some(Capability::unsupported(
                UnsupportedCode::Schema,
                "Base Output Time dataset absent; date stamps alone do not qualify mesh series",
            )));
        };
        Ok(if time.capability.is_supported() && time.shape.len() == 1 {
            None
        } else {
            Some(Capability::unsupported(
                UnsupportedCode::Schema,
                "Base Output Time is not a rank-one numeric dataset",
            ))
        })
    }
    fn mr_counts(&self, area: &str) -> Result<Vec<(ElementDomain, u64)>> {
        let mut counts = Vec::new();
        for (name, domain) in [
            ("Cells Center Coordinate", ElementDomain::Cell),
            ("Faces FacePoint Indexes", ElementDomain::Face),
            ("FacePoints Coordinate", ElementDomain::Facepoint),
        ] {
            if let Some(ds) = absent_ok(
                self.file
                    .dataset(&format!("{GEOM}/{area}/{name}"))
                    .map_err(map_error),
            )? {
                if ds.shape().len() != 2 || ds.shape()[1] != 2 {
                    return Err(Error::new(
                        ErrorKind::Corrupt,
                        "invalid mesh identity dimensions",
                    ));
                }
                counts.push((domain, ds.shape()[0]));
            }
        }
        Ok(counts)
    }
    fn mr_inventory(
        &self,
        area: &str,
        family: &str,
        bc: bool,
        max_bytes: usize,
    ) -> Result<MeshResultInventory> {
        self.memo(
            format!("mr_inventory:{:?}", (area, family, bc, max_bytes,)),
            || self.mr_inventory_uncached(area, family, bc, max_bytes),
        )
    }
    fn mr_inventory_uncached(
        &self,
        area: &str,
        family: &str,
        bc: bool,
        max_bytes: usize,
    ) -> Result<MeshResultInventory> {
        component(area)?;
        if absent_ok(self.group_members(&format!("{GEOM}/{area}")))?.is_none() {
            return Err(Error::new(ErrorKind::NotFound, "unknown mesh area"));
        }
        let group = format!(
            "{BASE}/{family}/2D Flow Areas/{area}{}",
            if bc { "/Boundary Conditions" } else { "" }
        );
        let Some(members) = absent_ok(self.group_members(&group))? else {
            return Ok(MeshResultInventory {
                group,
                capability: Capability::unavailable("canonical group absent"),
                variables: vec![],
            });
        };
        let counts = if bc { vec![] } else { self.mr_counts(area)? };
        let summary = family == "Summary Output";
        let times = if summary {
            None
        } else {
            Some(self.unsteady_time_axis(max_bytes)?.rows)
        };
        // PR #565 parity: a cell/face series needs the Base Output `Time` dataset itself
        // (rank one, numeric). An axis rebuilt from date stamps alone does not qualify it.
        let time_refusal = if summary || bc {
            None
        } else {
            self.mr_time_dataset_refusal()?
        };
        let mut variables = Vec::new();
        for name in members.datasets {
            let path = format!("{group}/{name}");
            let mut v = self.mr_variable(name, path, summary, bc, &counts, times, max_bytes)?;
            if let Some(refusal) = time_refusal
                .as_ref()
                .filter(|_| v.capability.is_supported())
            {
                v.capability = refusal.clone();
            }
            variables.push(v);
        }
        Ok(MeshResultInventory {
            group,
            capability: Capability::Supported,
            variables,
        })
    }
    /// Enumerate every directly stored summary dataset, retaining unqualified layouts.
    pub fn mesh_summary_inventory(
        &self,
        area: &str,
        max_bytes: usize,
    ) -> Result<MeshResultInventory> {
        self.mr_inventory(area, "Summary Output", false, max_bytes)
    }
    /// Return every stored summary row (value, optional time, or other explicitly labelled rows).
    pub fn read_mesh_summary(
        &self,
        area: &str,
        variable: &str,
        elements: Option<&[u64]>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<MeshResultSelection> {
        component(variable)?;
        let d = self.mr_descriptor(area, "Summary Output", variable, metadata_max_bytes)?;
        let (rows, _) = result_dimensions(&d)?;
        self.mr_read(&d, Slice::new(0, rows, 1), elements, max_bytes)
    }
    fn mr_read(
        &self,
        d: &MeshResultVariable,
        rows: Slice,
        elements: Option<&[u64]>,
        max_bytes: usize,
    ) -> Result<MeshResultSelection> {
        let values = self.mr_values(d, rows, elements, max_bytes)?;
        let (_, n) = result_dimensions(d)?;
        Ok(MeshResultSelection {
            id: Identity::new(self.path(), &d.path, 0),
            elements: elements.map_or_else(|| (0..n).collect(), |v| v.to_vec()),
            rows,
            values,
        })
    }
    fn mr_values(
        &self,
        d: &MeshResultVariable,
        rows: Slice,
        elements: Option<&[u64]>,
        max_bytes: usize,
    ) -> Result<NumericArray> {
        let (total_rows, n) = result_dimensions(d)?;
        let rank1 = d.shape.len() == 1;
        let nr = row_count(rows, total_rows)?;
        let ne = elements.map_or(n, |e| e.len() as u64);
        let k = d.kind.ok_or_else(|| schema("unproven numeric kind"))?;
        let bytes = nr
            .checked_mul(ne)
            .and_then(|n| n.checked_mul(width(k)))
            .ok_or_else(|| bad("result byte size overflow"))?;
        let fixed = bytes
            .checked_add(
                ne.checked_mul(8)
                    .ok_or_else(|| bad("identity size overflow"))?,
            )
            .ok_or_else(|| bad("working set overflow"))?;
        budget(fixed, max_bytes)?;
        let elems = elements.unwrap_or(&[]);
        if elems.iter().any(|&i| i >= n) {
            return Err(bad("mesh element out of bounds"));
        }
        let contiguous =
            elements.is_none() || elems.windows(2).all(|w| w[0].checked_add(1) == Some(w[1]));
        let read = |cols: Slice| {
            if rank1 {
                self.read_numeric(&d.path, Some(&[cols]))
            } else {
                self.read_numeric(&d.path, Some(&[rows, cols]))
            }
        };
        let values = if nr == 0 || ne == 0 {
            // Empty owned buffers without asking the generic reader to decode empty chunks.
            empty(k, if rank1 { vec![ne] } else { vec![nr, ne] })
        } else if contiguous {
            read(elements.map_or(Slice::new(0, n, 1), |e| {
                Slice::new(e[0], e[e.len() - 1] + 1, 1)
            }))?
        } else if !rank1 && max_bytes as u64 - fixed >= bytes {
            if let Some(v) = self.gather_numeric(&d.path, k, rows, elems)? {
                v
            } else {
                stream_gather(
                    k,
                    nr,
                    rank1,
                    elems,
                    d.chunks.as_ref().and_then(|c| c.last()).copied(),
                    max_bytes as u64 - fixed,
                    read,
                )?
            }
        } else {
            stream_gather(
                k,
                nr,
                rank1,
                elems,
                d.chunks.as_ref().and_then(|c| c.last()).copied(),
                max_bytes as u64 - fixed,
                read,
            )?
        };
        Ok(values)
    }
}
fn empty(k: NumericKind, shape: Vec<u64>) -> NumericArray {
    let values = match k {
        NumericKind::I8 => NumericValues::I8(vec![]),
        NumericKind::I16 => NumericValues::I16(vec![]),
        NumericKind::I32 => NumericValues::I32(vec![]),
        NumericKind::I64 => NumericValues::I64(vec![]),
        NumericKind::U8 => NumericValues::U8(vec![]),
        NumericKind::U16 => NumericValues::U16(vec![]),
        NumericKind::U32 => NumericValues::U32(vec![]),
        NumericKind::U64 => NumericValues::U64(vec![]),
        NumericKind::F32 => NumericValues::F32(vec![]),
        NumericKind::F64 => NumericValues::F64(vec![]),
    };
    NumericArray { shape, values }
}
#[allow(clippy::too_many_arguments)]
fn stream_gather(
    k: NumericKind,
    nr: u64,
    rank1: bool,
    elems: &[u64],
    chunk: Option<u32>,
    scratch: u64,
    read: impl Fn(Slice) -> Result<NumericArray>,
) -> Result<NumericArray> {
    let column_bytes = nr
        .checked_mul(width(k))
        .ok_or_else(|| bad("slab size overflow"))?;
    budget(column_bytes, scratch as usize)?;
    macro_rules! collect {
        ($v:ident) => {{
            let mut out = vec![Default::default(); nr as usize * elems.len()];
            let mut at = 0;
            while at < elems.len() {
                let start = at;
                let mut lo = elems[at];
                let mut hi = lo;
                at += 1;
                while at < elems.len() {
                    let next = elems[at];
                    let low = lo.min(next);
                    let high = hi.max(next);
                    let same_chunk =
                        chunk.is_some_and(|c| c > 0 && low / u64::from(c) == high / u64::from(c));
                    if !same_chunk
                        || (high - low + 1)
                            .checked_mul(column_bytes)
                            .is_none_or(|n| n > scratch)
                    {
                        break;
                    }
                    lo = low;
                    hi = high;
                    at += 1;
                }
                let slab = read(Slice::new(lo, hi + 1, 1))?;
                let NumericValues::$v(v) = slab.values else {
                    return Err(schema("numeric type changed"));
                };
                let span = (hi - lo + 1) as usize;
                for row in 0..nr as usize {
                    for col in start..at {
                        out[row * elems.len() + col] = v[row * span + (elems[col] - lo) as usize];
                    }
                }
                // slab dropped here before the next read: no collection of intermediate slabs.
            }
            NumericValues::$v(out)
        }};
    }
    let values = match k {
        NumericKind::I8 => collect!(I8),
        NumericKind::I16 => collect!(I16),
        NumericKind::I32 => collect!(I32),
        NumericKind::I64 => collect!(I64),
        NumericKind::U8 => collect!(U8),
        NumericKind::U16 => collect!(U16),
        NumericKind::U32 => collect!(U32),
        NumericKind::U64 => collect!(U64),
        NumericKind::F32 => collect!(F32),
        NumericKind::F64 => collect!(F64),
    };
    Ok(NumericArray {
        shape: if rank1 {
            vec![elems.len() as u64]
        } else {
            vec![nr, elems.len() as u64]
        },
        values,
    })
}
