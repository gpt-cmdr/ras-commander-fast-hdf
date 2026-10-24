//! Stored 1D cross-section results beyond Water Surface/Flow: every steady and unsteady XS
//! output variable present, including steady `Additional Variables`. Values are exact.
use super::metadata::TypedAttribute;
use super::structures::ColumnLabel;
use super::units::DatasetUnits;
use super::xs1d::{STEADY, UNSTEADY};
use super::xs_geometry::{
    budget, concat, corrupt, dataset_typed_attributes, declared_labels, invalid, take_rows,
    unsupported, width,
};
use super::{
    Identity, NumericArray, NumericKind, NumericValues, ReadOnlyHdfSession, ResultMode, Slice,
};
use crate::capability::{absent_ok, error_to_capability};
use crate::{Capability, Error, ErrorKind, Reported, Result, UnsupportedCode};

const IDENTITY_DATASETS: [&str; 2] = ["Cross Section Attributes", "Cross Section Only"];
const MAX_VARIABLES: usize = 4096;

/// Element layout of a stored 1D result variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum XsResultDomain {
    /// `(time/profile row, cross section)`.
    CrossSection,
    /// `(profile row, component, cross section)`, e.g. steady `Cross Section Variables`.
    CrossSectionComponents,
    /// No qualified element axis (inspect `capability`).
    Other,
}
impl XsResultDomain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CrossSection => "cross_section",
            Self::CrossSectionComponents => "cross_section_components",
            Self::Other => "other",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct XsResultVariable {
    /// Path relative to the `Cross Sections` group (`Velocity Channel`,
    /// `Additional Variables/Area Flow Total`).
    pub name: String,
    pub path: String,
    pub domain: XsResultDomain,
    pub shape: Vec<u64>,
    pub kind: Option<NumericKind>,
    pub chunks: Option<Vec<u32>>,
    pub capability: Capability,
    /// Every declared attribute (units, `Variable_Unit`, data-set extrema) with raw payloads.
    pub attributes: Vec<TypedAttribute>,
    /// Shared dataset-unit evidence (`Units`, `Variable Units`, `Variable`, quantity label).
    pub units: DatasetUnits,
    /// `Variable_Unit`/`Variables` name+unit pairs as declared (one per dataset for rank 2,
    /// one per component for rank 3).
    pub labels: Reported<Vec<ColumnLabel>>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct XsResultInventory {
    pub mode: ResultMode,
    pub group: String,
    /// Supported means enumeration and results-side identity validation succeeded; inspect
    /// each variable. Unavailable: the canonical group is absent.
    pub capability: Capability,
    /// Stored time rows (unsteady `Time` length) or steady profile count.
    pub rows: u64,
    /// Geometry identities of the result columns (validated River/Reach/Station order).
    pub sections: Vec<Identity>,
    pub variables: Vec<XsResultVariable>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct XsResultSelection {
    /// Dataset path and row 0.
    pub id: Identity,
    pub mode: ResultMode,
    pub variable: String,
    pub rows: Slice,
    /// Selected stored section columns (caller order, repeats kept).
    pub sections: Vec<u64>,
    /// Geometry identity of each selected column.
    pub section_ids: Vec<Identity>,
    /// Row-major `[rows, sections]` or `[rows, components, sections]`, stored type.
    pub values: NumericArray,
}

fn row_count(s: Slice, n: u64) -> Result<u64> {
    if s.step == 0 || s.start > s.end || s.end > n {
        return Err(invalid("invalid result row range"));
    }
    Ok((s.end - s.start).div_ceil(s.step))
}
fn mode_base(mode: ResultMode) -> &'static str {
    match mode {
        ResultMode::Steady => STEADY,
        ResultMode::Unsteady => UNSTEADY,
    }
}
/// Gather columns `cols` (indices into `width_cols`) of each row of a row-major
/// `[nrows, width_cols]` block. `inner` is the contiguous run length per column cell.
pub(super) fn gather_columns(
    values: &NumericValues,
    nrows: usize,
    width_cols: usize,
    inner: usize,
    cols: &[usize],
) -> NumericValues {
    let mut ranges = Vec::with_capacity(nrows * cols.len());
    for r in 0..nrows {
        for &c in cols {
            let start = (r * width_cols + c) * inner;
            ranges.push(start..start + inner);
        }
    }
    take_rows(values, 1, &ranges)
}
/// The stored `Dimension` attribute of a rank-3 result names its axes. The only qualified
/// layout (HEC-RAS 6.x steady `Cross Section Variables`) stores exactly
/// `[Profiles, Variables, Cross Sections]`; anything else (or absence) is unproven.
fn dimension_proven(attrs: &[TypedAttribute], mode: ResultMode) -> bool {
    let dims: Vec<&TypedAttribute> = attrs.iter().filter(|a| a.name == "Dimension").collect();
    mode == ResultMode::Steady
        && dims.len() == 1
        && matches!(&dims[0].value, super::metadata::MetadataValue::Strings(v)
            if v.len() == 3
                && v[0].trim() == "Profiles"
                && v[1].trim() == "Variables"
                && v[2].trim() == "Cross Sections")
}
impl ReadOnlyHdfSession {
    fn xs_result_rows(&self, mode: ResultMode, max_bytes: usize) -> Result<u64> {
        match mode {
            ResultMode::Steady => Ok(self.steady_profile_names(max_bytes)?.len() as u64),
            ResultMode::Unsteady => {
                let p = format!("{UNSTEADY}/Time");
                match absent_ok(self.file.dataset(&p).map_err(super::encoding::map_error))? {
                    Some(t) if t.shape().len() == 1 => Ok(t.shape()[0]),
                    Some(_) => Err(corrupt("unsteady Time axis must be rank one")),
                    None => Err(corrupt("unsteady XS results present without a Time axis")),
                }
            }
        }
    }
    /// Every stored XS result dataset below the canonical steady/unsteady `Cross Sections`
    /// group (identity tables excluded), each with its own capability, domain, attributes,
    /// units and labels. Rows are checked against `Time` / profile names, columns against the
    /// validated results-side identity. `max_bytes` bounds each consulted metadata object.
    pub fn xs_result_inventory(
        &self,
        mode: ResultMode,
        max_bytes: usize,
    ) -> Result<XsResultInventory> {
        self.memo(
            format!("xs_result_inventory:{:?}", (mode, max_bytes)),
            || {
                let group = format!("{}/Cross Sections", mode_base(mode));
                let mut inv = XsResultInventory {
                    mode,
                    group: group.clone(),
                    capability: Capability::Supported,
                    rows: 0,
                    sections: vec![],
                    variables: vec![],
                };
                if !self.has_group(&group)? {
                    inv.capability = Capability::unavailable("canonical XS result group absent");
                    return Ok(inv);
                }
                let setup = (|| -> Result<(u64, Vec<Identity>)> {
                    let geometry = self.cross_sections(max_bytes)?;
                    self.validate_identity(mode, &geometry, max_bytes)?;
                    let rows = self.xs_result_rows(mode, max_bytes)?;
                    Ok((rows, geometry.sections.into_iter().map(|s| s.id).collect()))
                })();
                let failure = match setup {
                    Ok((rows, ids)) => {
                        inv.rows = rows;
                        inv.sections = ids;
                        None
                    }
                    Err(e) => Some(error_to_capability(e)?),
                };
                let n = inv.sections.len() as u64;
                let mut pending = vec![(group.clone(), String::new(), 0usize)];
                while let Some((g, rel, depth)) = pending.pop() {
                    let m = self.group_members(&g)?;
                    for d in m.datasets {
                        if rel.is_empty() && IDENTITY_DATASETS.contains(&d.as_str()) {
                            continue;
                        }
                        if inv.variables.len() >= MAX_VARIABLES {
                            return Err(unsupported(
                                UnsupportedCode::ResourceLimit,
                                "XS result variable limit",
                            ));
                        }
                        let name = if rel.is_empty() {
                            d.clone()
                        } else {
                            format!("{rel}/{d}")
                        };
                        let path = format!("{g}/{d}");
                        let cap = self.numeric_capability(&path)?;
                        let attributes = dataset_typed_attributes(self, &path, max_bytes)?;
                        let units = self.dataset_units(&path)?;
                        let (domain, label_count) = match cap.shape.as_slice() {
                            [_, c] if *c == n && n > 0 => (XsResultDomain::CrossSection, 1),
                            [_, k, c] if *c == n && n > 0 => {
                                (XsResultDomain::CrossSectionComponents, *k)
                            }
                            _ => (XsResultDomain::Other, 1),
                        };
                        let capability = if let Some(f) = &failure {
                            f.clone()
                        } else if !cap.capability.is_supported() {
                            cap.capability.clone()
                        } else if domain == XsResultDomain::Other {
                            Capability::unsupported(
                                UnsupportedCode::IdentityLayout,
                                "no cross-section axis matching the validated identity count",
                            )
                        } else if domain == XsResultDomain::CrossSectionComponents
                            && !dimension_proven(&attributes, mode)
                        {
                            // Rank-3 orientation is taken from the stored `Dimension` attribute,
                            // never from shape alone. Only the steady layout is qualified.
                            Capability::unsupported(
                                UnsupportedCode::Schema,
                                "rank-3 orientation unproven: Dimension must be [Profiles, Variables, Cross Sections] (steady)",
                            )
                        } else if cap.shape[0] != inv.rows {
                            Capability::corrupt(format!(
                                "{path}: {} rows but {} time/profile rows",
                                cap.shape[0], inv.rows
                            ))
                        } else if cap.shape[0] == 0 {
                            Capability::unsupported(UnsupportedCode::Schema, "empty XS result rows")
                        } else {
                            Capability::Supported
                        };
                        // Velocity/lateral datasets carry quantity-label attributes instead (units).
                        let labels = declared_labels(&attributes, label_count);
                        inv.variables.push(XsResultVariable {
                            name,
                            path,
                            domain,
                            shape: cap.shape.clone(),
                            kind: cap.kind,
                            chunks: cap.chunks.clone(),
                            capability,
                            attributes,
                            units,
                            labels,
                        });
                    }
                    for sub in m.groups {
                        if depth >= 2 {
                            return Err(unsupported(
                                UnsupportedCode::ResourceLimit,
                                "XS result nesting limit",
                            ));
                        }
                        let r = if rel.is_empty() {
                            sub.clone()
                        } else {
                            format!("{rel}/{sub}")
                        };
                        pending.push((format!("{g}/{sub}"), r, depth + 1));
                    }
                }
                inv.variables.sort_by(|a, b| a.name.cmp(&b.name));
                if let Some(f) = failure {
                    inv.capability = f;
                }
                Ok(inv)
            },
        )
    }
    /// Exact selected rows (stored indices; see `unsteady_row_window` for a civil-time window)
    /// and section columns (None = all, caller order, repeats kept) of one XS result variable.
    /// `max_bytes` bounds returned values + identities and one transient covering slab.
    #[allow(clippy::too_many_arguments)]
    pub fn read_xs_result(
        &self,
        mode: ResultMode,
        variable: &str,
        rows: Slice,
        sections: Option<&[u64]>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<XsResultSelection> {
        let inv = self.xs_result_inventory(mode, metadata_max_bytes)?;
        let d = inv
            .variables
            .iter()
            .find(|v| v.name == variable)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "XS result variable absent"))?;
        match &d.capability {
            Capability::Supported => {}
            Capability::Unsupported { code, reason } => {
                return Err(unsupported(*code, reason.clone()))
            }
            c => return Err(corrupt(c.reason().unwrap_or("invalid XS result"))),
        }
        let n = inv.sections.len() as u64;
        let nr = row_count(rows, d.shape[0])?;
        let cols: Vec<u64> = match sections {
            Some(s) => {
                if let Some(bad) = s.iter().find(|&&c| c >= n) {
                    return Err(invalid(format!(
                        "section {bad} out of bounds ({n} sections)"
                    )));
                }
                s.to_vec()
            }
            None => (0..n).collect(),
        };
        let kind = d
            .kind
            .ok_or_else(|| unsupported(UnsupportedCode::Encoding, "unproven numeric kind"))?;
        let inner = if d.shape.len() == 3 { d.shape[1] } else { 1 };
        let returned = nr
            .checked_mul(cols.len() as u64)
            .and_then(|x| x.checked_mul(inner))
            .and_then(|x| x.checked_mul(width(kind)))
            .and_then(|x| x.checked_add(cols.len() as u64 * 8))
            .ok_or_else(|| invalid("selection size overflow"))?;
        budget(returned, max_bytes)?;
        let shape = if d.shape.len() == 3 {
            vec![nr, inner, cols.len() as u64]
        } else {
            vec![nr, cols.len() as u64]
        };
        let read = |lo: u64, hi: u64| -> Result<NumericArray> {
            let mut sel = vec![rows];
            if d.shape.len() == 3 {
                sel.push(Slice::new(0, inner, 1));
            }
            sel.push(Slice::new(lo, hi, 1));
            self.read_numeric(&d.path, Some(&sel))
        };
        let values = if nr == 0 || cols.is_empty() {
            concat(Vec::new(), kind)
        } else {
            let lo = *cols.iter().min().unwrap();
            let hi = *cols.iter().max().unwrap() + 1;
            let contiguous = cols.windows(2).all(|w| w[0] + 1 == w[1]);
            let span = nr * inner * (hi - lo) * width(kind);
            if contiguous {
                read(lo, hi)?.values
            } else if returned.saturating_add(span) <= max_bytes as u64 {
                let a = read(lo, hi)?;
                let rel: Vec<usize> = cols.iter().map(|&c| (c - lo) as usize).collect();
                // Rank 3 stores (row, component, section): gather per (row, component).
                gather_columns(
                    &a.values,
                    (nr * inner) as usize,
                    (hi - lo) as usize,
                    1,
                    &rel,
                )
            } else {
                let mut parts = Vec::new();
                for &c in &cols {
                    parts.push(read(c, c + 1)?.values);
                }
                // parts are [rows*inner] each in section order: interleave to row-major.
                let joined = concat(parts, kind);
                let k = cols.len();
                let per = (nr * inner) as usize;
                let mut ranges = Vec::with_capacity(per * k);
                for r in 0..per {
                    for c in 0..k {
                        let at = c * per + r;
                        ranges.push(at..at + 1);
                    }
                }
                take_rows(&joined, 1, &ranges)
            }
        };
        Ok(XsResultSelection {
            id: Identity::new(self.path(), &d.path, 0),
            mode,
            variable: variable.into(),
            rows,
            section_ids: cols
                .iter()
                .map(|&c| inv.sections[c as usize].clone())
                .collect(),
            sections: cols,
            values: NumericArray { shape, values },
        })
    }
    /// Stored unsteady rows whose civil time lies in `[start_ms, end_ms]` (milliseconds since
    /// the zone-less `CivilUnix1970` epoch of `unsteady_time_axis`). The validated axis is
    /// strictly increasing, so the window is one contiguous row slice (possibly empty).
    pub fn unsteady_row_window(
        &self,
        start_ms: i64,
        end_ms: i64,
        max_bytes: usize,
    ) -> Result<Slice> {
        if start_ms > end_ms {
            return Err(invalid("time window start after end"));
        }
        let axis = self.unsteady_time_axis(max_bytes)?;
        let ms = axis.ms_since_epoch.as_ref().ok_or_else(|| {
            unsupported(
                UnsupportedCode::Encoding,
                "time axis has no proven civil milliseconds",
            )
        })?;
        let first = ms.partition_point(|&t| t < start_ms) as u64;
        let last = ms.partition_point(|&t| t <= end_ms) as u64;
        Ok(Slice::new(first, last.max(first), 1))
    }
}
