//! Raw CRS evidence only. Text is never promoted to a validated semantic CRS.
//! Units/axes below are lexical evidence; no EPSG/datum/reprojection is inferred.
use crate::{Capability, UnsupportedCode};
use crate::{Error, ErrorKind, Result};
use std::path::{Component, Path, PathBuf};
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EvidenceStatus {
    Unknown,
    Unvalidated,
    Invalid,
    Conflict,
    /// Exact text is retained, but lexical evidence exceeds a bounded capability.
    Unsupported {
        code: UnsupportedCode,
        reason: String,
    },
    /// The stored attribute is damaged.
    Corrupt {
        reason: String,
    },
}
impl EvidenceStatus {
    pub fn unsupported(code: UnsupportedCode, reason: impl Into<String>) -> Self {
        Self::Unsupported {
            code,
            reason: reason.into(),
        }
    }
    pub fn corrupt(reason: impl Into<String>) -> Self {
        Self::Corrupt {
            reason: reason.into(),
        }
    }
    /// The one status vocabulary, derived from the lexical status (never stored beside it):
    /// Unknown is Unavailable (no text), Unvalidated is Supported (lexically readable, still
    /// not a validated CRS), Invalid is Corrupt, Conflict is Unsupported(Schema) because the
    /// declarations cannot be reconciled, and the payload variants map to themselves.
    pub fn capability(&self) -> Capability {
        match self {
            Self::Unknown => Capability::unavailable("no CRS text"),
            Self::Unvalidated => Capability::Supported,
            Self::Invalid => Capability::corrupt("CRS text is lexically malformed"),
            Self::Conflict => {
                Capability::unsupported(UnsupportedCode::Schema, "conflicting CRS declarations")
            }
            Self::Unsupported { code, reason } => Capability::unsupported(*code, reason.clone()),
            Self::Corrupt { reason } => Capability::corrupt(reason.clone()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CrsEvidence {
    pub source: String,
    pub exact_wkt: String,
    /// Exact HDF attribute payload where available.
    pub raw_bytes: Vec<u8>,
    pub unit_clauses: Vec<String>,
    pub axis_clauses: Vec<String>,
    pub status: EvidenceStatus,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CrsReport {
    pub evidence: Vec<CrsEvidence>,
    pub status: EvidenceStatus,
    pub hydraulic_units: Option<String>,
    /// Hydraulic units evidence is independent of projection validity.
    pub hydraulic_units_evidence: Vec<CrsEvidence>,
}
impl CrsEvidence {
    /// Status in the shared vocabulary (see [`EvidenceStatus::capability`]).
    pub fn capability(&self) -> Capability {
        self.status.capability()
    }
    pub fn raw(source: impl Into<String>, text: impl Into<String>) -> Self {
        let exact_wkt = text.into();
        let (status, unit_clauses, axis_clauses) = if exact_wkt.len() > MAX_WKT_BYTES {
            (
                EvidenceStatus::unsupported(
                    UnsupportedCode::ResourceLimit,
                    "CRS lexical input exceeds 1 MiB",
                ),
                Vec::new(),
                Vec::new(),
            )
        } else if exact_wkt.trim().is_empty() {
            (EvidenceStatus::Unknown, Vec::new(), Vec::new())
        } else {
            match lexical_clauses(&exact_wkt) {
                Ok((units, axes)) => (EvidenceStatus::Unvalidated, units, axes),
                Err(status) => (status, Vec::new(), Vec::new()),
            }
        };
        Self {
            source: source.into(),
            exact_wkt,
            raw_bytes: Vec::new(),
            unit_clauses,
            axis_clauses,
            status,
        }
    }
}
// One input pass and bounded output copying. Only doubled quotes escape a WKT
// quote; backslashes are literal. No semantic CRS interpretation is attempted.
const MAX_WKT_BYTES: usize = 1 << 20;
const MAX_WKT_DEPTH: usize = 128;
const MAX_CLAUSE_BYTES: usize = 1 << 20;
fn lexical_clauses(s: &str) -> std::result::Result<(Vec<String>, Vec<String>), EvidenceStatus> {
    let limit = |reason: &str| EvidenceStatus::unsupported(UnsupportedCode::ResourceLimit, reason);
    if s.len() > MAX_WKT_BYTES {
        return Err(limit("CRS lexical input exceeds 1 MiB"));
    }
    let bytes = s.as_bytes();
    let mut stack = Vec::new();
    // Reserve slots in opening order, so nested clauses preserve source order.
    let mut ranges: Vec<(usize, usize, bool)> = Vec::new();
    let mut quoted = false;
    let mut at = 0;
    let mut retained = 0usize;
    while at < bytes.len() {
        let c = bytes[at];
        if c == b'"' {
            if quoted && bytes.get(at + 1) == Some(&b'"') {
                at += 2;
                continue;
            }
            quoted = !quoted;
        } else if !quoted {
            if c == b'[' || c == b'(' {
                if stack.len() >= MAX_WKT_DEPTH {
                    return Err(limit("CRS lexical depth exceeds 128"));
                }
                let mut start = at;
                while start > 0
                    && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_')
                {
                    start -= 1;
                }
                let keyword = &s[start..at];
                let unit = [
                    "UNIT",
                    "ANGLEUNIT",
                    "LENGTHUNIT",
                    "SCALEUNIT",
                    "TIMEUNIT",
                    "PARAMETRICUNIT",
                ]
                .iter()
                .any(|key| keyword.eq_ignore_ascii_case(key));
                let axis = keyword.eq_ignore_ascii_case("AXIS");
                let slot = if unit || axis {
                    let slot = ranges.len();
                    ranges.push((start, 0, axis));
                    Some(slot)
                } else {
                    None
                };
                stack.push((c, slot));
            } else if c == b']' || c == b')' {
                let Some((open, slot)) = stack.pop() else {
                    return Err(EvidenceStatus::Invalid);
                };
                if (open == b'[') != (c == b']') {
                    return Err(EvidenceStatus::Invalid);
                }
                if let Some(slot) = slot {
                    let range = &mut ranges[slot];
                    range.1 = at + 1;
                    retained = retained
                        .checked_add(range.1 - range.0)
                        .ok_or_else(|| limit("CRS lexical clause byte overflow"))?;
                    if retained > MAX_CLAUSE_BYTES {
                        return Err(limit("CRS lexical clauses exceed 1 MiB"));
                    }
                }
            }
        }
        at += 1;
    }
    if quoted || !stack.is_empty() {
        return Err(EvidenceStatus::Invalid);
    }
    let mut units = Vec::new();
    let mut axes = Vec::new();
    for (start, end, axis) in ranges {
        if axis {
            axes.push(s[start..end].to_owned());
        } else {
            units.push(s[start..end].to_owned());
        }
    }
    Ok((units, axes))
}
impl CrsReport {
    /// Status in the shared vocabulary (see [`EvidenceStatus::capability`]).
    pub fn capability(&self) -> Capability {
        self.status.capability()
    }
}
pub fn combine(evidence: Vec<CrsEvidence>, hydraulic_units: Option<String>) -> CrsReport {
    let nonempty: Vec<_> = evidence
        .iter()
        .filter(|e| e.status != EvidenceStatus::Unknown)
        .collect();
    let status = if nonempty.is_empty() {
        EvidenceStatus::Unknown
    } else if let Some(e) = nonempty.iter().find(|e| {
        matches!(
            e.status,
            EvidenceStatus::Unsupported { .. } | EvidenceStatus::Corrupt { .. }
        )
    }) {
        e.status.clone()
    } else if nonempty.iter().any(|e| e.status == EvidenceStatus::Invalid) {
        EvidenceStatus::Invalid
    } else if nonempty
        .iter()
        .any(|e| e.exact_wkt != nonempty[0].exact_wkt)
    {
        EvidenceStatus::Conflict
    } else {
        EvidenceStatus::Unvalidated
    };
    CrsReport {
        evidence,
        status,
        hydraulic_units,
        hydraulic_units_evidence: Vec::new(),
    }
}
/// Caller supplies a projection-file reference obtained from project/rasmap
/// records. Automatic rasmap XML/entity resolution is not implemented.
/// Relative references are resolved against the catalog source's parent.
pub fn relative_projection(
    project: impl AsRef<Path>,
    reference: impl AsRef<Path>,
) -> Result<CrsEvidence> {
    let (project, reference) = (project.as_ref(), reference.as_ref());
    if reference.is_absolute()
        || reference.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return Err(Error::new(
            ErrorKind::InvalidRequest,
            "projection reference must stay project-relative",
        ));
    }
    let path: PathBuf = project
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::InvalidRequest, "project has no parent"))?
        .join(reference);
    use std::io::Read;
    let file = std::fs::File::open(&path).map_err(|e| {
        Error::new(
            if e.kind() == std::io::ErrorKind::NotFound {
                ErrorKind::FileNotFound
            } else {
                ErrorKind::Io
            },
            e.to_string(),
        )
        .with_path(path.clone())
    })?;
    let mut bytes = Vec::new();
    file.take((MAX_WKT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::new(ErrorKind::Io, e.to_string()).with_path(path.clone()))?;
    if bytes.len() > MAX_WKT_BYTES {
        return Err(Error::unsupported(
            UnsupportedCode::ResourceLimit,
            "projection file exceeds 1 MiB",
        )
        .with_path(path));
    }
    // SourceDocument policy: UTF8/BOM, then named Latin1 fallback; UTF16/NUL refused.
    if bytes.contains(&0) || bytes.starts_with(&[255, 254]) || bytes.starts_with(&[254, 255]) {
        return Err(
            Error::unsupported(UnsupportedCode::Encoding, "UTF16/NUL projection source")
                .with_path(path),
        );
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_owned(),
        Err(_) => bytes.iter().map(|b| char::from(*b)).collect(),
    };
    if text.len() > MAX_WKT_BYTES {
        return Err(Error::unsupported(
            UnsupportedCode::ResourceLimit,
            "decoded projection exceeds 1 MiB",
        )
        .with_path(path));
    }
    let mut evidence = CrsEvidence::raw(path.to_string_lossy(), text);
    evidence.raw_bytes = bytes;
    Ok(evidence)
}
#[cfg(feature = "hdf")]
impl crate::hdf::ReadOnlyHdfSession {
    pub fn raw_crs_evidence(&self) -> Result<CrsReport> {
        use crate::hdf::metadata::MetadataValue;
        let attrs = self.typed_group_attributes("/")?;
        let mut evidence = Vec::new();
        let mut hydraulic = None;
        let mut units_evidence = Vec::new();
        for a in attrs {
            if a.name == "Projection" {
                match a.value {
                    MetadataValue::Strings(values) => {
                        for text in values {
                            let mut e = CrsEvidence::raw(
                                format!("{}:/@Projection", self.path().display()),
                                text,
                            );
                            e.raw_bytes = a.raw_bytes.clone();
                            evidence.push(e);
                        }
                    }
                    value => {
                        let mut e =
                            CrsEvidence::raw(format!("{}:/@Projection", self.path().display()), "");
                        e.raw_bytes = a.raw_bytes;
                        e.status = match value {
                            MetadataValue::Corrupt { reason } => EvidenceStatus::corrupt(reason),
                            MetadataValue::Unsupported { code, reason } => {
                                EvidenceStatus::unsupported(code, reason)
                            }
                            _ => EvidenceStatus::unsupported(
                                UnsupportedCode::Encoding,
                                "Projection attribute is not decoded text; inspect raw attribute bytes",
                            ),
                        };
                        evidence.push(e);
                    }
                }
            }
        }
        // `Units System` is read once, by the shared units reader, so CRS and units agree.
        let system = self.unit_system_evidence("/")?;
        match system.value.value() {
            Some(text) => hydraulic = Some(text.clone()),
            None if system.value.is_unavailable() => {}
            None => {
                let mut e =
                    CrsEvidence::raw(format!("{}:/@Units System", self.path().display()), "");
                e.status = match system.value.capability() {
                    Capability::Corrupt { reason } => EvidenceStatus::corrupt(reason.clone()),
                    Capability::Unsupported { code, reason } => {
                        EvidenceStatus::unsupported(*code, reason.clone())
                    }
                    other => EvidenceStatus::unsupported(
                        UnsupportedCode::Schema,
                        other
                            .reason()
                            .unwrap_or("Units System is not one decoded string"),
                    ),
                };
                e.raw_bytes = system
                    .value
                    .raw_bytes()
                    .map(<[u8]>::to_vec)
                    .unwrap_or_default();
                units_evidence.push(e);
            }
        }
        let mut report = combine(evidence, hydraulic);
        report.hydraulic_units_evidence = units_evidence;
        Ok(report)
    }
}
#[cfg(feature = "catalog")]
pub fn catalog_projection(
    catalog: &crate::catalog::ProjectCatalog,
    reference: impl AsRef<Path>,
) -> Result<CrsEvidence> {
    relative_projection(&catalog.source.path, reference)
}
