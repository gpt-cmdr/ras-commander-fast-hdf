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
    /// Source file, `Geometry/2D Flow Areas/Attributes` and this mesh's attribute row.
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
    /// Group discovery is available with hdf alone. Names-only layouts remain unsupported.
    pub fn mesh_names(&self) -> Result<Vec<String>> {
        let groups = self.group_members(GEOMETRY)?.groups;
        if groups.is_empty() {
            return Err(Error::unsupported(
                UnsupportedCode::NotImplemented,
                "no mesh groups; legacy Names-only inventory not yet implemented",
            ));
        }
        Ok(groups
            .into_iter()
            .map(|n| n.rsplit('/').next().unwrap_or(&n).to_owned())
            .collect())
    }
    fn mesh_attributes(&self, max_bytes: usize) -> Result<(Vec<String>, Vec<i64>)> {
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
            let (names, _) = self.mesh_attributes(max_bytes)?;
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
    /// Source-exact counts require Attributes/group agreement. Name mismatches fail
    /// with all structured diagnostics in mesh_name_diagnostics(), never an empty pass.
    pub fn mesh_inventory(&self, max_bytes: usize) -> Result<Vec<MeshDescriptor>> {
        self.memo(format!("mesh_inventory:{:?}", (max_bytes,)), || {
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
                        format!("{GEOMETRY}/Attributes"),
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
