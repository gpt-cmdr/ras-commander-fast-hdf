//! Bounded mesh descriptors, explicit name diagnostics and output-block discovery.
use crate::capability::error_to_capability;
use crate::hdf::{metadata::MetadataValue, GroupMembers, ReadOnlyHdfSession, ResultMode};
use crate::{Capability, Error, ErrorKind, Result, UnsupportedCode};
use std::collections::{BTreeSet, HashMap, HashSet};
const GEOMETRY: &str = "Geometry/2D Flow Areas";
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VariableDescriptor {
    pub dataset: String,
    pub shape: Vec<u64>,
    pub capability: Capability,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResultAvailability {
    pub capability: Capability,
    pub variables: Vec<VariableDescriptor>,
}
/// Deprecated name of the shared [`crate::hdf::ResultMode`].
#[deprecated(note = "use hdf::ResultMode")]
pub type MeshResultMode = crate::hdf::ResultMode;
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MeshOutputBlock {
    pub mode: ResultMode,
    pub block_name: String,
    pub dataset_root: String,
    pub results: ResultAvailability,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MeshNameIssue {
    MissingGroup,
    ExtraGroup,
    DuplicateAttributeName,
    InvalidName,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MeshNameDiagnostic {
    pub mesh_name: String,
    pub issue: MeshNameIssue,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MeshDescriptor {
    /// Source file, inventory dataset (`Attributes` or legacy `Names`) and stored row.
    pub id: crate::hdf::Identity,
    pub mesh_name: String,
    pub physical_cells: u64,
    pub stored_cells: u64,
    pub ghost_cells: u64,
    /// Compatibility view: Unsteady / Base Output only.
    pub results: ResultAvailability,
    /// All present blocks plus explicit absent/unsupported family states.
    pub output_blocks: Vec<MeshOutputBlock>,
}
fn empty(capability: Capability) -> ResultAvailability {
    ResultAvailability {
        capability,
        variables: Vec::new(),
    }
}
/// A failed availability probe: damaged or unsupported storage is a block-level status; `NotFound`
/// and non-budget `InvalidRequest` are not canonical absence here and propagate unchanged.
fn availability_failure(e: Error) -> Result<ResultAvailability> {
    if e.kind == ErrorKind::NotFound {
        return Err(e);
    }
    Ok(empty(error_to_capability(e)?))
}
fn diagnostics(names: &[String], groups: &[String]) -> Vec<MeshNameDiagnostic> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let group_set: HashSet<&str> = groups.iter().map(String::as_str).collect();
    let attribute_set: HashSet<&str> = names.iter().map(String::as_str).collect();
    for name in names {
        let issue = if name.is_empty() || name.contains(['/', '\\']) {
            Some(MeshNameIssue::InvalidName)
        } else if !seen.insert(name) {
            Some(MeshNameIssue::DuplicateAttributeName)
        } else if !group_set.contains(name.as_str()) {
            Some(MeshNameIssue::MissingGroup)
        } else {
            None
        };
        if let Some(issue) = issue {
            out.push(MeshNameDiagnostic {
                mesh_name: name.clone(),
                issue,
            });
        }
    }
    for name in groups {
        if !attribute_set.contains(name.as_str()) {
            out.push(MeshNameDiagnostic {
                mesh_name: name.clone(),
                issue: MeshNameIssue::ExtraGroup,
            });
        }
    }
    out
}
impl ReadOnlyHdfSession {
    fn mesh_has_attributes(&self) -> Result<bool> {
        let members = self.group_members(GEOMETRY)?;
        if members.groups.iter().any(|name| name == "Attributes") {
            return Err(Error::new(ErrorKind::Corrupt, "mesh Attributes is a group"));
        }
        Ok(members.datasets.iter().any(|name| name == "Attributes"))
    }
    /// The legacy inventory is a stored rank-one fixed-string dataset, in row order.
    pub(crate) fn legacy_mesh_names(&self, max_bytes: usize) -> Result<Vec<String>> {
        let path = format!("{GEOMETRY}/Names");
        let raw = self.raw_fixed_strings(&path, max_bytes)?;
        let Some(value) = raw.value() else {
            return Err(match raw.capability() {
                Capability::Unavailable { reason } => Error::new(ErrorKind::NotFound, reason),
                Capability::Corrupt { reason } => Error::new(ErrorKind::Corrupt, reason),
                c => Error::unsupported(
                    c.code().unwrap_or(UnsupportedCode::Encoding),
                    c.reason().unwrap_or("unproven Names dataset"),
                ),
            });
        };
        if value.shape.len() != 1 {
            return Err(Error::new(
                ErrorKind::Corrupt,
                "mesh Names must be rank one",
            ));
        }
        // RAS 5.0.x stores blank-padded names even with a NULLPAD datatype.
        let names = self
            .read_strings(&path)?
            .into_iter()
            .map(|name| name.trim_end_matches(' ').to_owned())
            .collect::<Vec<_>>();
        if names
            .iter()
            .any(|name| name.is_empty() || name.contains(['/', '\\']))
        {
            return Err(Error::new(ErrorKind::Corrupt, "invalid legacy mesh name"));
        }
        Ok(names)
    }

    /// Qualify the legacy physical prefix from stored cell-face ranges. A polygon cell
    /// has at least three faces; one-face boundary ghosts must form a trailing suffix.
    /// Neither result width nor the stale `Cell Maximum Index` attribute is evidence.
    pub(crate) fn legacy_mesh_physical_count(&self, area: &str, max_bytes: usize) -> Result<u64> {
        let root = format!("{GEOMETRY}/{area}");
        let centers = self.numeric_capability(&format!("{root}/Cells Center Coordinate"))?;
        let faces =
            self.numeric_capability(&format!("{root}/Cells Face and Orientation Values"))?;
        let info = self.read_numeric_bounded(
            &format!("{root}/Cells Face and Orientation Info"),
            None,
            max_bytes,
        )?;
        if centers.shape.len() != 2
            || centers.shape[1] != 2
            || info.shape != centers.shape
            || faces.shape.len() != 2
            || faces.shape[1] != 2
        {
            return Err(Error::new(
                ErrorKind::Corrupt,
                "legacy cell-face tables must align with N by 2 centers",
            ));
        }
        if !centers.capability.is_supported() || !faces.capability.is_supported() {
            return Err(Error::unsupported(
                UnsupportedCode::Encoding,
                "legacy centers or cell-face values use an unsupported numeric encoding",
            ));
        }
        fn count(mut values: impl Iterator<Item = i128>, face_rows: u64) -> Result<u64> {
            let mut end = 0i128;
            let mut physical = 0u64;
            let mut ghosts = false;
            while let Some(start) = values.next() {
                let n = values
                    .next()
                    .ok_or_else(|| Error::new(ErrorKind::Corrupt, "incomplete cell-face row"))?;
                if start != end || n < 1 || start + n > i128::from(face_rows) {
                    return Err(Error::new(
                        ErrorKind::Corrupt,
                        "invalid legacy cell-face range",
                    ));
                }
                if n == 1 {
                    ghosts = true;
                } else if n < 3 || ghosts {
                    return Err(Error::unsupported(
                        UnsupportedCode::IdentityLayout,
                        "legacy physical/ghost cell boundary is unqualified",
                    ));
                } else {
                    physical += 1;
                }
                end = start + n;
            }
            if end != i128::from(face_rows) || physical == 0 {
                return Err(Error::new(
                    ErrorKind::Corrupt,
                    "legacy cell-face ranges do not cover stored values",
                ));
            }
            Ok(physical)
        }
        use crate::hdf::NumericValues as V;
        macro_rules! count {
            ($v:expr) => {
                count($v.into_iter().map(i128::from), faces.shape[0])
            };
        }
        match info.values {
            V::I8(v) => count!(v),
            V::I16(v) => count!(v),
            V::I32(v) => count!(v),
            V::I64(v) => count!(v),
            V::U8(v) => count!(v),
            V::U16(v) => count!(v),
            V::U32(v) => count!(v),
            V::U64(v) => count!(v),
            _ => Err(Error::unsupported(
                UnsupportedCode::Encoding,
                "legacy cell-face Info requires integers",
            )),
        }
    }
    /// Group discovery is available with hdf alone, independently of inventory metadata.
    pub fn mesh_names(&self) -> Result<Vec<String>> {
        let groups = self.group_members(GEOMETRY)?.groups;
        if groups.is_empty() {
            return Err(Error::unsupported(
                UnsupportedCode::NotImplemented,
                "no mesh groups; group discovery is unavailable",
            ));
        }
        Ok(groups
            .into_iter()
            .map(|n| n.rsplit('/').next().unwrap_or(&n).to_owned())
            .collect())
    }
    fn mesh_attributes(&self, max_bytes: usize) -> Result<(Vec<String>, Vec<i64>)> {
        if !self.mesh_has_attributes()? {
            let names = self.legacy_mesh_names(max_bytes)?;
            let counts = names
                .iter()
                .map(|name| {
                    i64::try_from(self.legacy_mesh_physical_count(name, max_bytes)?)
                        .map_err(|_| Error::new(ErrorKind::Corrupt, "physical count exceeds i64"))
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok((names, counts));
        }
        let table = self.simple_compound(&format!("{GEOMETRY}/Attributes"), max_bytes)?;
        let names = table
            .columns
            .iter()
            .find(|c| c.name == "Name")
            .and_then(|c| match &c.value {
                MetadataValue::Strings(v) => Some(v.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                Error::unsupported(
                    UnsupportedCode::Encoding,
                    "mesh Name requires proven fixed strings",
                )
            })?;
        let counts = table
            .columns
            .iter()
            .find(|c| c.name == "Cell Count")
            .and_then(|c| match &c.value {
                MetadataValue::Signed(v) => Some(Ok(v.clone())),
                MetadataValue::Unsigned(v) => Some(
                    v.iter()
                        .map(|&n| i64::try_from(n))
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(|_| Error::new(ErrorKind::Corrupt, "Cell Count exceeds i64")),
                ),
                _ => None,
            })
            .ok_or_else(|| {
                Error::unsupported(
                    UnsupportedCode::Encoding,
                    "Cell Count requires a proven integer column",
                )
            })??;
        if names.len() != counts.len() {
            return Err(Error::new(
                ErrorKind::Corrupt,
                "mesh attribute column lengths differ",
            ));
        }
        Ok((names, counts))
    }
    /// Inspect all missing/extra/duplicate/invalid names without reading center arrays.
    pub fn mesh_name_diagnostics(&self, max_bytes: usize) -> Result<Vec<MeshNameDiagnostic>> {
        self.memo(format!("mesh_name_diagnostics:{:?}", (max_bytes,)), || {
            let names = if self.mesh_has_attributes()? {
                self.mesh_attributes(max_bytes)?.0
            } else {
                self.legacy_mesh_names(max_bytes)?
            };
            let groups = self.group_members(GEOMETRY)?.groups;
            Ok(diagnostics(&names, &groups))
        })
    }
    // A missing component is absence; a dataset used as a group is corruption.
    // Do not turn all reader NotFound errors into absence after path traversal.
    fn inventory_group(
        &self,
        path: &str,
        cache: &mut HashMap<String, Option<GroupMembers>>,
    ) -> Result<Option<GroupMembers>> {
        if let Some(value) = cache.get(path) {
            return Ok(value.clone());
        }
        let mut current = String::new();
        let mut members = match cache.get("/") {
            Some(Some(m)) => m.clone(),
            _ => self.group_members("/")?,
        };
        cache.insert("/".into(), Some(members.clone()));
        for part in path.trim_matches('/').split('/') {
            if members.datasets.iter().any(|n| n == part) {
                return Err(Error::new(
                    ErrorKind::Corrupt,
                    format!("dataset used as inventory group: {current}/{part}"),
                ));
            }
            if !members.groups.iter().any(|n| n == part) {
                cache.insert(path.into(), None);
                return Ok(None);
            }
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
            members = match cache.get(&current) {
                Some(Some(m)) => m.clone(),
                _ => self.group_members(&current)?,
            };
            cache.insert(current.clone(), Some(members.clone()));
        }
        Ok(Some(members))
    }
    fn mesh_result_availability(
        &self,
        path: &str,
        steady: bool,
        cache: &mut HashMap<String, Option<GroupMembers>>,
    ) -> Result<ResultAvailability> {
        let members = match self.inventory_group(path, cache) {
            Ok(Some(m)) => m,
            Ok(None) => {
                return Ok(empty(if steady {
                    Capability::unsupported(
                        UnsupportedCode::Schema,
                        format!("steady results present, qualified mesh path absent: {path}"),
                    )
                } else {
                    Capability::unavailable(format!("mesh result path absent: {path}"))
                }))
            }
            Err(e) => return availability_failure(e),
        };
        let variables = members
            .datasets
            .into_iter()
            .map(|name| {
                let dataset = format!("{path}/{name}");
                let c = self.numeric_capability(&dataset)?;
                Ok(VariableDescriptor {
                    dataset,
                    shape: c.shape,
                    capability: c.capability,
                })
            })
            .collect::<Result<Vec<_>>>();
        let variables = match variables {
            Ok(v) => v,
            Err(e) => return availability_failure(e),
        };
        Ok(ResultAvailability {
            capability: Capability::Supported,
            variables,
        })
    }
    fn mesh_output_blocks(
        &self,
        name: &str,
        cache: &mut HashMap<String, Option<GroupMembers>>,
    ) -> Result<Vec<MeshOutputBlock>> {
        let mut out = Vec::new();
        for (mode, family, series) in [
            (ResultMode::Unsteady, "Unsteady", "Unsteady Time Series"),
            (ResultMode::Steady, "Steady", "Steady Profiles"),
        ] {
            let family_path = format!("Results/{family}");
            let family_present = self.inventory_group(&family_path, cache)?.is_some();
            let root = format!("{family_path}/Output/Output Blocks");
            let blocks = if family_present {
                self.inventory_group(&root, cache)?
            } else {
                None
            };
            let names = blocks
                .as_ref()
                .map(|m| {
                    m.groups
                        .iter()
                        .chain(m.datasets.iter())
                        .cloned()
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            if names.is_empty() {
                let reason = if family_present {
                    format!("{family} results present; no supported output-block groups")
                } else {
                    format!("{family_path} absent")
                };
                out.push(MeshOutputBlock {
                    mode,
                    block_name: "Base Output".into(),
                    dataset_root: format!("{root}/Base Output/{series}/2D Flow Areas/{name}"),
                    results: empty(if family_present && mode == ResultMode::Steady {
                        Capability::unsupported(UnsupportedCode::Schema, reason)
                    } else {
                        Capability::unavailable(reason)
                    }),
                });
            } else {
                for block_name in names {
                    let dataset_root = format!("{root}/{block_name}/{series}/2D Flow Areas/{name}");
                    let results = self.mesh_result_availability(
                        &dataset_root,
                        mode == ResultMode::Steady,
                        cache,
                    )?;
                    out.push(MeshOutputBlock {
                        mode,
                        block_name,
                        dataset_root,
                        results,
                    });
                }
            }
        }
        Ok(out)
    }
    /// Counts require inventory/group agreement and a qualified physical-cell count.
    /// Names mismatches fail
    /// with all structured diagnostics in mesh_name_diagnostics(), never an empty pass.
    pub fn mesh_inventory(&self, max_bytes: usize) -> Result<Vec<MeshDescriptor>> {
        self.memo(format!("mesh_inventory:{:?}", (max_bytes,)), || {
            let inventory = if self.mesh_has_attributes()? {
                "Attributes"
            } else {
                "Names"
            };
            let (names, counts) = self.mesh_attributes(max_bytes)?;
            let groups = self.group_members(GEOMETRY)?.groups;
            let issues = diagnostics(&names, &groups);
            if !issues.is_empty() {
                return Err(Error::new(
                    ErrorKind::Corrupt,
                    format!("mesh Attributes/group mismatch: {issues:?}"),
                ));
            }
            if names.is_empty() && groups.is_empty() {
                return Err(Error::unsupported(
                    UnsupportedCode::Schema,
                    "no mesh rows or groups; inventory is unavailable",
                ));
            }
            let mut group_cache = HashMap::new();
            let mut out = Vec::new();
            for (row, (name, count)) in names.into_iter().zip(counts).enumerate() {
                let centers =
                    self.numeric_capability(&format!("{GEOMETRY}/{name}/Cells Center Coordinate"))?;
                if centers.shape.len() != 2 || centers.shape[1] != 2 {
                    return Err(Error::new(ErrorKind::Corrupt, "centers must be N by 2"));
                }
                if !centers.capability.is_supported() {
                    return Err(Error::unsupported(
                        UnsupportedCode::Encoding,
                        "center coordinates require proven numeric encoding",
                    ));
                }
                let stored = centers.shape[0];
                let physical = u64::try_from(count)
                    .map_err(|_| Error::new(ErrorKind::Corrupt, "negative physical cell count"))?;
                let ghosts = stored.checked_sub(physical).ok_or_else(|| {
                    Error::new(ErrorKind::Corrupt, "physical count exceeds storage")
                })?;
                let output_blocks = self.mesh_output_blocks(&name, &mut group_cache)?;
                let results = output_blocks
                    .iter()
                    .find(|b| b.mode == ResultMode::Unsteady && b.block_name == "Base Output")
                    .map(|b| b.results.clone())
                    .unwrap_or_else(|| {
                        empty(Capability::unavailable("Unsteady Base Output block absent"))
                    });
                out.push(MeshDescriptor {
                    id: crate::hdf::Identity::new(
                        self.path(),
                        format!("{GEOMETRY}/{inventory}"),
                        row as u64,
                    ),
                    mesh_name: name,
                    physical_cells: physical,
                    stored_cells: stored,
                    ghost_cells: ghosts,
                    results,
                    output_blocks,
                });
            }
            Ok(out)
        })
    }
    #[deprecated(note = "use mesh_inventory(max_bytes); this alias uses DEFAULT_MAX_BYTES")]
    pub fn initial_2d_inventory(&self) -> Result<Vec<MeshDescriptor>> {
        self.mesh_inventory(crate::hdf::DEFAULT_MAX_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_failures_propagate_unless_status_material() {
        for kind in [
            ErrorKind::NotFound,
            ErrorKind::InvalidRequest,
            ErrorKind::Io,
        ] {
            assert_eq!(
                availability_failure(Error::new(kind, "x"))
                    .unwrap_err()
                    .kind,
                kind
            );
        }
        let budget =
            Error::new(ErrorKind::InvalidRequest, "b").with_code(UnsupportedCode::ResourceLimit);
        assert!(!availability_failure(budget)
            .unwrap()
            .capability
            .is_supported());
        assert!(matches!(
            availability_failure(Error::new(ErrorKind::Corrupt, "c"))
                .unwrap()
                .capability,
            Capability::Corrupt { .. }
        ));
        assert_eq!(
            availability_failure(Error::unsupported(UnsupportedCode::Encoding, "e"))
                .unwrap()
                .capability
                .code(),
            Some(UnsupportedCode::Encoding)
        );
    }
}
