//! 1D and structure siblings of the 2D Summary Output reader: every stored dataset below the
//! unsteady Base Output `Summary Output` groups other than `2D Flow Areas`, with element
//! domains, declared labels, proven row labels and time-of-extreme rows. Values are exact.
use super::metadata::{MetadataValue, TypedAttribute};
use super::structures::ColumnLabel;
use super::xs_geometry::{
    budget, concat, corrupt, dataset_typed_attributes, declared_labels, invalid, take_rows,
    unsupported, width,
};
use super::xs_results::gather_columns;
use super::{Identity, NumericArray, NumericKind, ReadOnlyHdfSession, ResultMode};
use crate::capability::{absent_ok, error_to_capability};
use crate::{Capability, Error, ErrorKind, Reported, Result, UnsupportedCode};

const SUMMARY: &str = "Results/Unsteady/Output/Output Blocks/Base Output/Summary Output";
const MAX_VARIABLES: usize = 4096;

/// Element domain of a 1D summary variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OneDElementDomain {
    CrossSection,
    /// Column count equals the storage-area count; the column-to-area order is NOT asserted.
    StorageArea,
    Other,
}
impl OneDElementDomain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CrossSection => "cross_section",
            Self::StorageArea => "storage_area",
            Self::Other => "other",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OneDSummaryVariable {
    /// Group directly below `Summary Output` (`Cross Sections`, `Storage Areas`, ...).
    pub family: String,
    /// Path relative to the family group.
    pub name: String,
    pub path: String,
    pub shape: Vec<u64>,
    pub kind: Option<NumericKind>,
    pub chunks: Option<Vec<u32>>,
    pub domain: OneDElementDomain,
    /// Stored axis that indexes elements (None when no qualified element axis).
    pub element_axis: Option<usize>,
    pub capability: Capability,
    pub attributes: Vec<TypedAttribute>,
    /// Declared `Variable_Unit` / `Variables` pairs, any count, exactly as stored.
    pub labels: Reported<Vec<ColumnLabel>>,
    /// Per stored row labels when proven: declared pairs whose count equals the non-element
    /// row count, or `Row0..` / `Row N:  name` attributes.
    pub row_labels: Option<Vec<ColumnLabel>>,
    /// Rows whose label starts with `Time` (time of the extreme, stored days, not converted).
    pub time_of_extreme_rows: Vec<u64>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OneDSummaryInventory {
    pub group: String,
    pub capability: Capability,
    pub variables: Vec<OneDSummaryVariable>,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OneDSummarySelection {
    pub id: Identity,
    pub family: String,
    pub name: String,
    /// Selected element indices along `element_axis` (all stored elements when None).
    pub elements: Vec<u64>,
    /// Geometry identities, only for the cross-section domain.
    pub element_ids: Option<Vec<Identity>>,
    /// Stored layout with the element axis subset; every non-element row is returned.
    pub values: NumericArray,
}

fn label_rows(labels: &[ColumnLabel]) -> Vec<u64> {
    labels
        .iter()
        .enumerate()
        .filter(|(_, l)| l.name.trim_start().starts_with("Time"))
        .map(|(i, _)| i as u64)
        .collect()
}
/// `Row0`..`RowN` (names, optional shared `Units`) or `Row N:  name` = unit attributes.
fn attribute_row_labels(attrs: &[TypedAttribute], rows: u64) -> Option<Vec<ColumnLabel>> {
    let text = |a: &TypedAttribute| match &a.value {
        MetadataValue::Strings(v) if v.len() == 1 => Some(v[0].clone()),
        _ => None,
    };
    let units = attrs.iter().find(|a| a.name == "Units").and_then(text);
    let mut out: Vec<Option<ColumnLabel>> = vec![None; rows as usize];
    let mut any = false;
    for a in attrs {
        if let Some(i) = a
            .name
            .strip_prefix("Row")
            .and_then(|r| r.parse::<usize>().ok())
        {
            if i < out.len() {
                out[i] = Some(ColumnLabel {
                    name: text(a)?,
                    unit: units.clone(),
                });
                any = true;
            }
        } else if let Some((idx, name)) =
            a.name.strip_prefix("Row ").and_then(|r| r.split_once(':'))
        {
            let i = idx.trim().parse::<usize>().ok()?;
            if i < out.len() {
                out[i] = Some(ColumnLabel {
                    name: name.trim().to_string(),
                    unit: text(a),
                });
                any = true;
            }
        }
    }
    if !any {
        return None;
    }
    out.into_iter().collect()
}

impl ReadOnlyHdfSession {
    /// Every stored dataset below the unsteady Base Output `Summary Output` groups other than
    /// `2D Flow Areas` (cross sections, storage areas and any structure families present),
    /// with element domain, declared labels, proven row labels and time-of-extreme rows.
    pub fn one_d_summary_inventory(&self, max_bytes: usize) -> Result<OneDSummaryInventory> {
        self.memo(format!("one_d_summary_inventory:{max_bytes}"), || {
            let mut inv = OneDSummaryInventory {
                group: SUMMARY.into(),
                capability: Capability::Supported,
                variables: vec![],
            };
            let Some(members) = absent_ok(self.group_members(SUMMARY))? else {
                inv.capability = Capability::unavailable("Summary Output absent");
                return Ok(inv);
            };
            if let Some(d) = members.datasets.first() {
                return Err(corrupt(format!(
                    "{SUMMARY}/{d}: dataset where a family group is required"
                )));
            }
            let xs = (|| -> Result<Vec<Identity>> {
                let g = self.cross_sections(max_bytes)?;
                self.validate_identity(ResultMode::Unsteady, &g, max_bytes)?;
                Ok(g.sections.into_iter().map(|s| s.id).collect())
            })();
            let sa = (|| -> Result<u64> {
                let p = "Geometry/Storage Areas/Attributes";
                let ds = self.file.dataset(p).map_err(super::encoding::map_error)?;
                Ok(ds.shape().first().copied().unwrap_or(0))
            })();
            let mut families = members.groups;
            families.sort();
            for family in families {
                if family == "2D Flow Areas" {
                    continue;
                }
                let (count, identity_failure): (Option<u64>, Option<Capability>) =
                    match family.as_str() {
                        "Cross Sections" => match &xs {
                            Ok(ids) => (Some(ids.len() as u64), None),
                            Err(e) => (None, Some(error_to_capability(e.clone())?)),
                        },
                        "Storage Areas" => match &sa {
                            Ok(n) => (Some(*n), None),
                            Err(e) if e.kind == ErrorKind::NotFound => (
                                None,
                                Some(Capability::unsupported(
                                    UnsupportedCode::IdentityLayout,
                                    "storage-area attribute table absent",
                                )),
                            ),
                            Err(e) => (None, Some(error_to_capability(e.clone())?)),
                        },
                        _ => (None, None),
                    };
                let domain_kind = match family.as_str() {
                    "Cross Sections" => OneDElementDomain::CrossSection,
                    "Storage Areas" => OneDElementDomain::StorageArea,
                    _ => OneDElementDomain::Other,
                };
                let mut pending = vec![(format!("{SUMMARY}/{family}"), String::new(), 0usize)];
                while let Some((g, rel, depth)) = pending.pop() {
                    let m = self.group_members(&g)?;
                    for d in m.datasets {
                        if inv.variables.len() >= MAX_VARIABLES {
                            return Err(unsupported(
                                UnsupportedCode::ResourceLimit,
                                "summary variable limit",
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
                        let declared = attributes
                            .iter()
                            .find(|a| matches!(a.name.as_str(), "Variable_Unit" | "Variables"))
                            .map(|a| a.shape.first().copied().unwrap_or(0));
                        let labels = match declared {
                            Some(k) => declared_labels(&attributes, k),
                            None => Reported::unavailable("no declared name/unit pairs"),
                        };
                        let shape = cap.shape.clone();
                        // Element axis: the axis whose length is the element count; when both
                        // axes match, declared per-row labels decide (rows on axis 0).
                        let axis = count.and_then(|n| match shape.as_slice() {
                            [a] if *a == n => Some(0),
                            [a, b] if *a == n && *b == n => match declared {
                                Some(k) if k == *a && k > 1 => Some(1),
                                _ => None,
                            },
                            [_, b] if *b == n => Some(1),
                            [a, _] if *a == n => Some(0),
                            _ => None,
                        });
                        let rows = match (axis, shape.as_slice()) {
                            (Some(1), [r, _]) => *r,
                            _ => 1,
                        };
                        let row_labels = if axis == Some(1) {
                            match (declared, labels.value()) {
                                (Some(k), Some(l)) if k == rows => Some(l.clone()),
                                _ => attribute_row_labels(&attributes, rows),
                            }
                        } else {
                            None
                        };
                        let time_of_extreme_rows =
                            row_labels.as_deref().map(label_rows).unwrap_or_default();
                        let capability = if !cap.capability.is_supported() {
                            cap.capability.clone()
                        } else if let Some(f) = &identity_failure {
                            f.clone()
                        } else if domain_kind != OneDElementDomain::Other && axis.is_none() {
                            Capability::unsupported(
                                UnsupportedCode::IdentityLayout,
                                "no unique element axis matching the element count",
                            )
                        } else if domain_kind == OneDElementDomain::Other {
                            // Structure families below Summary Output: no real fixture writes
                            // one, so the layout is reported, never read as Supported.
                            Capability::unsupported(
                                UnsupportedCode::Schema,
                                format!("unqualified layout: no real fixture qualifies Summary Output/{family}"),
                            )
                        } else if shape.is_empty() || shape.len() > 2 {
                            Capability::unsupported(
                                UnsupportedCode::Schema,
                                "unqualified summary rank",
                            )
                        } else {
                            Capability::Supported
                        };
                        inv.variables.push(OneDSummaryVariable {
                            family: family.clone(),
                            name,
                            path,
                            shape,
                            kind: cap.kind,
                            chunks: cap.chunks.clone(),
                            domain: if axis.is_some() {
                                domain_kind
                            } else {
                                OneDElementDomain::Other
                            },
                            element_axis: axis,
                            capability,
                            attributes,
                            labels,
                            row_labels,
                            time_of_extreme_rows,
                        });
                    }
                    for sub in m.groups {
                        if depth >= 3 {
                            return Err(unsupported(
                                UnsupportedCode::ResourceLimit,
                                "summary nesting limit",
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
            }
            inv.variables
                .sort_by(|a, b| (&a.family, &a.name).cmp(&(&b.family, &b.name)));
            Ok(inv)
        })
    }
    /// Every stored row of one 1D summary variable with the element axis subset (None = all
    /// elements, caller order, repeats kept). Variables without a qualified element axis are
    /// readable only with `elements = None`. `max_bytes` bounds returned bytes.
    pub fn read_one_d_summary(
        &self,
        family: &str,
        name: &str,
        elements: Option<&[u64]>,
        metadata_max_bytes: usize,
        max_bytes: usize,
    ) -> Result<OneDSummarySelection> {
        let inv = self.one_d_summary_inventory(metadata_max_bytes)?;
        let d = inv
            .variables
            .iter()
            .find(|v| v.family == family && v.name == name)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "summary variable absent"))?;
        let kind = d
            .kind
            .ok_or_else(|| unsupported(UnsupportedCode::Encoding, "unproven numeric kind"))?;
        let full_ok = elements.is_none()
            && d.capability.code() == Some(UnsupportedCode::IdentityLayout)
            && d.domain == OneDElementDomain::Other;
        match &d.capability {
            Capability::Supported => {}
            _ if full_ok => {}
            Capability::Unsupported { code, reason } => {
                return Err(unsupported(*code, reason.clone()))
            }
            c => return Err(corrupt(c.reason().unwrap_or("invalid summary variable"))),
        }
        let total: u64 = d.shape.iter().product();
        let Some(axis) = d.element_axis else {
            if elements.is_some() {
                return Err(invalid("variable has no qualified element axis"));
            }
            budget(total * width(kind), max_bytes)?;
            let a = if total == 0 {
                NumericArray {
                    shape: d.shape.clone(),
                    values: concat(Vec::new(), kind),
                }
            } else {
                self.read_numeric(&d.path, None)?
            };
            return Ok(OneDSummarySelection {
                id: Identity::new(self.path(), &d.path, 0),
                family: family.into(),
                name: name.into(),
                elements: vec![],
                element_ids: None,
                values: a,
            });
        };
        let n = d.shape[axis];
        let sel: Vec<u64> = match elements {
            Some(e) => {
                if let Some(bad) = e.iter().find(|&&i| i >= n) {
                    return Err(invalid(format!(
                        "element {bad} out of bounds ({n} elements)"
                    )));
                }
                e.to_vec()
            }
            None => (0..n).collect(),
        };
        let other = if d.shape.len() == 2 {
            d.shape[1 - axis]
        } else {
            1
        };
        budget(
            (other * sel.len() as u64 * width(kind)).saturating_add(total * width(kind)),
            max_bytes,
        )?;
        let full = if total == 0 {
            NumericArray {
                shape: d.shape.clone(),
                values: concat(Vec::new(), kind),
            }
        } else {
            self.read_numeric(&d.path, None)?
        };
        let cols: Vec<usize> = sel.iter().map(|&i| i as usize).collect();
        let (shape, values) = match (d.shape.len(), axis) {
            (1, _) => (
                vec![sel.len() as u64],
                take_rows(
                    &full.values,
                    1,
                    &cols.iter().map(|&c| c..c + 1).collect::<Vec<_>>(),
                ),
            ),
            (_, 1) => (
                vec![other, sel.len() as u64],
                gather_columns(&full.values, other as usize, n as usize, 1, &cols),
            ),
            _ => (
                vec![sel.len() as u64, other],
                take_rows(
                    &full.values,
                    other as usize,
                    &cols.iter().map(|&c| c..c + 1).collect::<Vec<_>>(),
                ),
            ),
        };
        let element_ids = if d.domain == OneDElementDomain::CrossSection {
            let g = self.cross_sections(metadata_max_bytes)?;
            Some(
                sel.iter()
                    .map(|&i| g.sections[i as usize].id.clone())
                    .collect(),
            )
        } else {
            None
        };
        Ok(OneDSummarySelection {
            id: Identity::new(self.path(), &d.path, 0),
            family: family.into(),
            name: name.into(),
            elements: sel,
            element_ids,
            values: NumericArray { shape, values },
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn row_label_attributes_parse_both_conventions() {
        let a = |name: &str, v: &str| TypedAttribute {
            name: name.into(),
            shape: vec![],
            dataspace: crate::hdf::Dataspace::Scalar,
            attribute_type: crate::hdf::AttributeType::Compound,
            datatype_label: String::new(),
            raw_bytes: vec![],
            value: MetadataValue::Strings(vec![v.into()]),
        };
        let l =
            attribute_row_labels(&[a("Row 0:  WSEL", "ft"), a("Row 1:  Time", "days")], 2).unwrap();
        assert_eq!(label_rows(&l), vec![1]);
        assert_eq!(l[0].unit.as_deref(), Some("ft"));
        let l = attribute_row_labels(
            &[
                a("Row0", "Initial"),
                a("Row1", "First"),
                a("Row2", "Difference"),
                a("Units", "cfs"),
            ],
            3,
        )
        .unwrap();
        assert!(label_rows(&l).is_empty());
        assert_eq!(l[2].unit.as_deref(), Some("cfs"));
        assert!(attribute_row_labels(&[a("Row0", "x")], 2).is_none());
    }
}
