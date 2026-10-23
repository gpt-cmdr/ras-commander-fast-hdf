//! Read-only HEC-RAS project and plan inventory, independent of HDF and Python.
//!
//! Record interpretation follows ras-commander `RasPrj`/`RasPlan` at
//! 4476c709105824cc25a5859b5005e3e545f56390 (MIT). Original bytes are retained;
//! no solver, editing, CRS inference, or result-completeness assertion is made.

use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Stable catalog failure category. Missing linked files are inventory states.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CatalogErrorKind {
    NotFound,
    Io,
    Malformed,
    UnsupportedEncoding,
    AmbiguousPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CatalogError {
    pub kind: CatalogErrorKind,
    pub path: PathBuf,
    pub message: String,
}
impl CatalogError {
    pub fn new(
        kind: CatalogErrorKind,
        path: impl Into<PathBuf>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            path: path.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?}: {}: {}",
            self.kind,
            self.path.display(),
            self.message
        )
    }
}
impl std::error::Error for CatalogError {}
type Result<T> = std::result::Result<T, CatalogError>;
fn malformed(path: &Path, message: impl Into<String>) -> CatalogError {
    CatalogError {
        kind: CatalogErrorKind::Malformed,
        path: path.into(),
        message: message.into(),
    }
}
fn io_error(path: &Path, error: std::io::Error) -> CatalogError {
    CatalogError {
        kind: if error.kind() == std::io::ErrorKind::NotFound {
            CatalogErrorKind::NotFound
        } else {
            CatalogErrorKind::Io
        },
        path: path.into(),
        message: error.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextEncoding {
    Utf8,
    Latin1,
}

/// All original source bytes and top-level records, including unknown records.
/// Description-block text is retained in bytes but never treated as control keys.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourceDocument {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub encoding: TextEncoding,
    pub records: Vec<Record>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Record {
    pub line: usize,
    pub key: String,
    pub value: Option<String>,
}

impl SourceDocument {
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|e| io_error(path, e))?;
        Self::from_bytes(path, bytes)
    }
    pub fn from_bytes(path: impl AsRef<Path>, bytes: Vec<u8>) -> Result<Self> {
        let path = path.as_ref();
        if bytes.contains(&0)
            || bytes.starts_with(&[0xff, 0xfe])
            || bytes.starts_with(&[0xfe, 0xff])
        {
            return Err(CatalogError {
                kind: CatalogErrorKind::UnsupportedEncoding,
                path: path.into(),
                message: "UTF-16/NUL-containing source is unsupported".into(),
            });
        }
        let (text, encoding) = match std::str::from_utf8(&bytes) {
            Ok(text) => (
                text.trim_start_matches('\u{feff}').to_owned(),
                TextEncoding::Utf8,
            ),
            // Match Python's UTF-8 then Latin-1 fallback; do not silently claim CP1252.
            Err(_) => (
                bytes.iter().map(|&b| char::from(b)).collect(),
                TextEncoding::Latin1,
            ),
        };
        let mut records = Vec::new();
        let mut description = false;
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.eq_ignore_ascii_case("BEGIN DESCRIPTION:")
                || line.eq_ignore_ascii_case("BEGIN DESCRIPTION")
            {
                if description {
                    return Err(malformed(path, "nested description block"));
                }
                description = true;
                continue;
            }
            if line.eq_ignore_ascii_case("END DESCRIPTION:")
                || line.eq_ignore_ascii_case("END DESCRIPTION")
            {
                if !description {
                    return Err(malformed(path, "description end without start"));
                }
                description = false;
                continue;
            }
            if description || line.is_empty() {
                continue;
            }
            let (key, value) = match line.split_once('=') {
                Some((key, value)) => (key.trim(), Some(value.trim().to_owned())),
                None => (line, None),
            };
            records.push(Record {
                line: index + 1,
                key: key.into(),
                value,
            });
        }
        if description {
            return Err(malformed(path, "unterminated description block"));
        }
        Ok(Self {
            path: path.into(),
            bytes,
            encoding,
            records,
        })
    }
    pub fn values<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.records
            .iter()
            .filter(move |r| r.key.eq_ignore_ascii_case(key))
            .filter_map(|r| r.value.as_deref())
    }
    fn optional(&self, key: &str) -> Result<Option<String>> {
        let mut values = self.values(key);
        let first = values.next();
        if values.any(|next| Some(next) != first) {
            return Err(malformed(
                &self.path,
                format!("conflicting {key} declarations"),
            ));
        }
        Ok(first.filter(|v| !v.is_empty()).map(str::to_owned))
    }
    fn required(&self, key: &str) -> Result<String> {
        self.optional(key)?
            .ok_or_else(|| malformed(&self.path, format!("missing {key}")))
    }
}

/// Project unit declarations do not establish a CRS or distinguish foot definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProjectUnits {
    English,
    Si,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Availability {
    Present,
    Missing,
    NotFile,
    /// Rejected before filesystem access, including absolute/network references.
    OutsideProject,
    /// Malformed or unreadable declaration; inspect diagnostic.
    InvalidReference,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourceLink {
    pub declared: String,
    pub path: PathBuf,
    pub availability: Availability,
    pub case_adjusted: bool,
    pub diagnostic: Option<CatalogError>,
}

// Only ASCII case differences are accepted. Unicode uppercase/casefold tables
// do not define NTFS identity and can alias distinct sibling paths.
fn path_case_eq(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    match (a.to_str(), b.to_str()) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => a == b,
    }
}
// Resolve Windows-authored separators on any host. Preserve the literal declaration
// separately. Resolve case only when each filesystem component has one match.
fn resolve_path(path: &Path) -> Result<(PathBuf, bool)> {
    let mut resolved = PathBuf::new();
    let mut changed = false;
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                let exact = resolved.join(name);
                let folder = if resolved.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    &resolved
                };
                let entries = match fs::read_dir(folder) {
                    Ok(entries) => entries,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        resolved = exact;
                        continue;
                    }
                    Err(error) => return Err(io_error(folder, error)),
                };
                let mut matches = Vec::new();
                let mut exact_match = None;
                for entry in entries {
                    let entry = entry.map_err(|e| io_error(folder, e))?;
                    if entry.file_name() == name {
                        exact_match = Some(entry.path());
                    }
                    if path_case_eq(&entry.file_name(), name) {
                        matches.push(entry.path());
                    }
                }
                if let Some(exact) = exact_match {
                    resolved = exact;
                    continue;
                }
                match matches.len() {
                    0 => resolved = exact,
                    1 => {
                        resolved = matches.remove(0);
                        changed = true;
                    }
                    _ => {
                        return Err(CatalogError {
                            kind: CatalogErrorKind::AmbiguousPath,
                            path: path.into(),
                            message: "multiple case-insensitive path matches".into(),
                        })
                    }
                }
            }
            other => resolved.push(other.as_os_str()),
        }
    }
    Ok((resolved, changed))
}
fn link_for(path: PathBuf, declared: &str) -> Result<SourceLink> {
    let (path, case_adjusted) = resolve_path(&path)?;
    let availability = match fs::metadata(&path) {
        Ok(info) if info.is_file() => Availability::Present,
        Ok(_) => Availability::NotFile,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Availability::Missing,
        Err(error) => return Err(io_error(&path, error)),
    };
    Ok(SourceLink {
        declared: declared.into(),
        path,
        availability,
        case_adjusted,
        diagnostic: None,
    })
}
// Lexical containment only: symlinks/junctions are not resolved. No filesystem
// call occurs for rooted, prefixed, network or escaping declarations.
fn declared_link(source: &Path, root: &Path, declaration: &str, prefixes: &[u8]) -> SourceLink {
    let blocked = |availability, message: &str| SourceLink {
        declared: declaration.into(),
        path: PathBuf::from(declaration),
        availability,
        case_adjusted: false,
        diagnostic: Some(malformed(source, message)),
    };
    let normalized = declaration.replace('\\', "/");
    let value = normalized.trim();
    if value.starts_with('/')
        || value.as_bytes().get(1) == Some(&b':')
        || Path::new(value)
            .components()
            .any(|c| matches!(c, Component::RootDir | Component::Prefix(_)))
    {
        return blocked(
            Availability::OutsideProject,
            "rooted/prefixed reference is outside project policy",
        );
    }
    let attempt = || -> Result<SourceLink> {
        let bytes = value.as_bytes();
        let suffix = bytes.len() >= 2
            && prefixes.contains(&bytes[0].to_ascii_lowercase())
            && bytes[1..].iter().all(u8::is_ascii_digit);
        let candidate = if suffix {
            source.with_extension(value)
        } else {
            let ext = Path::new(value)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .as_bytes();
            if ext.len() < 2
                || !prefixes.contains(&ext[0].to_ascii_lowercase())
                || !ext[1..].iter().all(u8::is_ascii_digit)
            {
                return Err(malformed(source, "invalid file reference"));
            }
            source.parent().unwrap_or(Path::new(".")).join(value)
        };
        let root = std::path::absolute(if root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            root
        })
        .map_err(|e| io_error(root, e))?;
        let absolute = std::path::absolute(&candidate).map_err(|e| io_error(&candidate, e))?;
        let mut clean = PathBuf::new();
        for c in absolute.components() {
            match c {
                Component::ParentDir => {
                    clean.pop();
                }
                Component::CurDir => {}
                _ => clean.push(c.as_os_str()),
            }
        }
        // Windows path casing is not identity. Compare components with the same
        // ASCII-only case policy used by resolve_path, without probing untrusted targets.
        let mut components = clean.components();
        let contained = root.components().all(|r| {
            components.next().is_some_and(|c| {
                if cfg!(windows) {
                    path_case_eq(c.as_os_str(), r.as_os_str())
                } else {
                    c == r
                }
            })
        });
        if !contained {
            return Ok(blocked(
                Availability::OutsideProject,
                "file reference escapes project folder",
            ));
        }
        link_for(clean, declaration)
    };
    attempt().unwrap_or_else(|error| SourceLink {
        diagnostic: Some(error),
        ..blocked(Availability::InvalidReference, "invalid file reference")
    })
}
fn hdf_link(source: &Path) -> SourceLink {
    let mut value = source.as_os_str().to_os_string();
    value.push(".hdf");
    let path = PathBuf::from(value);
    link_for(path.clone(), "derived: append .hdf").unwrap_or_else(|error| SourceLink {
        declared: "derived: append .hdf".into(),
        path,
        availability: Availability::InvalidReference,
        case_adjusted: false,
        diagnostic: Some(error),
    })
}

/// A model-local civil declaration, deliberately not a UTC/nanosecond timestamp.
/// 2400 is retained as hour 24, without changing the declared calendar day.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeclaredDateTime {
    pub raw_date: String,
    pub raw_time: String,
    pub year: u32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}
#[allow(clippy::manual_is_multiple_of)] // Keep the catalog usable before Rust 1.87.
fn date_time(path: &Path, date: &str, time: &str) -> Result<DeclaredDateTime> {
    let date = date.trim();
    let time = time.trim();
    let compact_time = if time.len() == 5 && time.as_bytes()[2] == b':' {
        time.replace(':', "")
    } else {
        time.to_owned()
    };
    if !date.is_ascii()
        || date.len() < 8
        || compact_time.len() != 4
        || !compact_time.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(malformed(path, "invalid Simulation Date date/time form"));
    }
    let month_start = date
        .find(|c: char| c.is_ascii_alphabetic())
        .ok_or_else(|| malformed(path, "missing month name"))?;
    if !(1..=2).contains(&month_start) || date.len() < month_start + 7 {
        return Err(malformed(path, "invalid date form"));
    }
    let day: u8 = date[..month_start]
        .parse()
        .map_err(|_| malformed(path, "invalid day"))?;
    let month_name = date[month_start..month_start + 3].to_ascii_uppercase();
    let month = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ]
    .iter()
    .position(|m| *m == month_name)
    .map(|m| (m + 1) as u8)
    .ok_or_else(|| malformed(path, "invalid month"))?;
    let year: u32 = date[month_start + 3..]
        .parse()
        .map_err(|_| malformed(path, "invalid year"))?;
    let hour: u8 = compact_time[..2]
        .parse()
        .map_err(|_| malformed(path, "invalid hour"))?;
    let minute: u8 = compact_time[2..]
        .parse()
        .map_err(|_| malformed(path, "invalid minute"))?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if year == 0
        || day == 0
        || day > days
        || hour > 24
        || minute > 59
        || (hour == 24 && minute != 0)
    {
        return Err(malformed(path, "date/time out of range"));
    }
    Ok(DeclaredDateTime {
        raw_date: date.into(),
        raw_time: time.into(),
        year,
        month,
        day,
        hour,
        minute,
    })
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SimulationWindow {
    pub raw: String,
    pub start: DeclaredDateTime,
    pub end: DeclaredDateTime,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IntervalUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DeclaredInterval {
    pub raw: String,
    pub value: IntervalValue,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum IntervalValue {
    Duration {
        amount: f64,
        unit: IntervalUnit,
    },
    /// A maximum profile is a result selection, not a duration.
    MaximumProfile,
}
fn interval(path: &Path, raw: String) -> Result<DeclaredInterval> {
    if raw.eq_ignore_ascii_case("Max Profile") {
        return Ok(DeclaredInterval {
            raw,
            value: IntervalValue::MaximumProfile,
        });
    }
    let split = raw
        .find(|c: char| c.is_ascii_alphabetic())
        .ok_or_else(|| malformed(path, format!("invalid interval: {raw}")))?;
    let amount: f64 = raw[..split]
        .trim()
        .parse()
        .map_err(|_| malformed(path, format!("invalid interval: {raw}")))?;
    let unit = match raw[split..].trim().to_ascii_uppercase().as_str() {
        "SEC" | "SECOND" | "SECONDS" => IntervalUnit::Second,
        "MIN" | "MINUTE" | "MINUTES" => IntervalUnit::Minute,
        "HOUR" | "HOURS" | "HR" => IntervalUnit::Hour,
        "DAY" | "DAYS" => IntervalUnit::Day,
        "WEEK" | "WEEKS" => IntervalUnit::Week,
        "MON" | "MONTH" | "MONTHS" => IntervalUnit::Month,
        "YEAR" | "YEARS" | "YR" => IntervalUnit::Year,
        _ => return Err(malformed(path, format!("unknown interval unit: {raw}"))),
    };
    if !amount.is_finite() || amount <= 0.0 {
        return Err(malformed(path, "interval must be positive and finite"));
    }
    Ok(DeclaredInterval {
        raw,
        value: IntervalValue::Duration { amount, unit },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FlowKind {
    Steady,
    Unsteady,
    QuasiUnsteady,
    /// No valid flow kind can be established from the declaration.
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FlowSource {
    pub kind: FlowKind,
    pub link: SourceLink,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Plan {
    pub source: SourceDocument,
    pub title: String,
    pub short_identifier: Option<String>,
    pub program_version: Option<String>,
    pub geometry: Option<SourceLink>,
    pub geometry_hdf: Option<SourceLink>,
    pub flow: Option<FlowSource>,
    /// Presence alone does not establish successful computation or valid HDF.
    pub results_hdf: SourceLink,
    pub simulation: Option<SimulationWindow>,
    pub intervals: Vec<(String, DeclaredInterval)>,
}
impl Plan {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_document(SourceDocument::read(path)?)
    }
    pub fn from_document(source: SourceDocument) -> Result<Self> {
        let root = source.path.parent().unwrap_or(Path::new(".")).to_path_buf();
        Self::from_document_in_root(source, &root)
    }
    fn from_document_in_root(source: SourceDocument, root: &Path) -> Result<Self> {
        let title = source.required("Plan Title")?;
        let short_identifier = source.optional("Short Identifier")?;
        let program_version = source.optional("Program Version")?;
        let geometry = source
            .optional("Geom File")?
            .map(|v| declared_link(&source.path, root, &v, b"g"));
        let geometry_hdf = geometry.as_ref().map(|g| {
            if matches!(
                g.availability,
                Availability::OutsideProject | Availability::InvalidReference
            ) {
                g.clone()
            } else {
                hdf_link(&g.path)
            }
        });
        let flow = source
            .optional("Flow File")?
            .map(|v| {
                let link = declared_link(&source.path, root, &v, b"fuq");
                let extension = link
                    .path
                    .extension()
                    .and_then(|v| v.to_str())
                    .unwrap_or_default();
                let kind = match extension.as_bytes().first().map(u8::to_ascii_lowercase) {
                    Some(b'f') => FlowKind::Steady,
                    Some(b'u') => FlowKind::Unsteady,
                    Some(b'q') if link.availability != Availability::InvalidReference => {
                        FlowKind::QuasiUnsteady
                    }
                    _ => FlowKind::Unknown,
                };
                let kind = if link.availability == Availability::InvalidReference {
                    FlowKind::Unknown
                } else {
                    kind
                };
                Ok(FlowSource { kind, link })
            })
            .transpose()?;
        let results_hdf = hdf_link(&source.path);
        let simulation = source
            .optional("Simulation Date")?
            .map(|raw| {
                let parts: Vec<_> = raw.split(',').collect();
                if parts.len() != 4 {
                    return Err(malformed(
                        &source.path,
                        "Simulation Date requires four comma-delimited values",
                    ));
                }
                let start = date_time(&source.path, parts[0], parts[1])?;
                let end = date_time(&source.path, parts[2], parts[3])?;
                Ok(SimulationWindow { raw, start, end })
            })
            .transpose()?;
        let mut intervals = Vec::new();
        for key in [
            "Computation Interval",
            "Output Interval",
            "Mapping Interval",
            "Instantaneous Interval",
        ] {
            if let Some(raw) = source.optional(key)? {
                intervals.push((key.into(), interval(&source.path, raw)?));
            }
        }
        Ok(Self {
            source,
            title,
            short_identifier,
            program_version,
            geometry,
            geometry_hdf,
            flow,
            results_hdf,
            simulation,
            intervals,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PlanEntry {
    pub link: SourceLink,
    /// Missing, unreadable and malformed listed plans remain explicit errors.
    pub plan: std::result::Result<Plan, CatalogError>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProjectDiagnostic {
    CurrentPlanNotListed {
        declared: String,
    },
    DuplicatePlan {
        declared: String,
    },
    LinkError {
        declared: String,
        error: CatalogError,
    },
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ProjectCatalog {
    pub source: SourceDocument,
    pub title: String,
    pub program_version: Option<String>,
    pub units: ProjectUnits,
    pub current_plan: Option<SourceLink>,
    pub plans: Vec<PlanEntry>,
    pub geometries: Vec<SourceLink>,
    pub steady_flows: Vec<SourceLink>,
    pub unsteady_flows: Vec<SourceLink>,
    pub quasi_unsteady_flows: Vec<SourceLink>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}
impl ProjectCatalog {
    /// Read only explicitly listed source files, retaining partial inventory on plan errors.
    /// Paths resolve relative to the project/plan file; no recursive model discovery occurs.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let source = SourceDocument::read(path)?;
        let title = source.required("Proj Title")?;
        let program_version = source.optional("Program Version")?;
        let mut units = ProjectUnits::Unknown;
        for record in &source.records {
            let candidate =
                if record.key.eq_ignore_ascii_case("English Units") && record.value.is_none() {
                    Some(ProjectUnits::English)
                } else if record.key.eq_ignore_ascii_case("SI Units") {
                    match record
                        .value
                        .as_deref()
                        .map(str::to_ascii_lowercase)
                        .as_deref()
                    {
                        None | Some("true" | "1" | "yes") => Some(ProjectUnits::Si),
                        Some("false" | "0" | "no") => Some(ProjectUnits::English),
                        _ => return Err(malformed(&source.path, "unknown SI Units declaration")),
                    }
                } else {
                    None
                };
            if let Some(candidate) = candidate {
                if units != ProjectUnits::Unknown && candidate != units {
                    return Err(malformed(&source.path, "conflicting project units"));
                }
                units = candidate;
            }
        }
        let root = source.path.parent().unwrap_or(Path::new("."));
        let links = |key: &str, prefixes: &[u8]| -> Vec<SourceLink> {
            source
                .values(key)
                .map(|v| declared_link(&source.path, root, v, prefixes))
                .collect()
        };
        let plan_links = links("Plan File", b"p");
        let current_plan = source
            .optional("Current Plan")?
            .map(|v| declared_link(&source.path, root, &v, b"p"));
        let mut diagnostics = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for link in &plan_links {
            let number = link
                .path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            if !seen.insert(number) {
                diagnostics.push(ProjectDiagnostic::DuplicatePlan {
                    declared: link.declared.clone(),
                });
            }
        }

        if let Some(current) = &current_plan {
            if !plan_links.iter().any(|p| p.path == current.path) {
                diagnostics.push(ProjectDiagnostic::CurrentPlanNotListed {
                    declared: current.declared.clone(),
                });
            }
        }
        let plans: Vec<PlanEntry> = plan_links
            .into_iter()
            .map(|link| {
                let plan = if let Some(error) = &link.diagnostic {
                    Err(error.clone())
                } else {
                    SourceDocument::read(&link.path)
                        .and_then(|s| Plan::from_document_in_root(s, root))
                };
                PlanEntry { link, plan }
            })
            .collect();
        let geometries = links("Geom File", b"g");
        let steady_flows = links("Flow File", b"f");
        let unsteady_flows = links("Unsteady File", b"u");
        let quasi_unsteady_flows = links("QuasiUnsteady File", b"q");
        for link in plans
            .iter()
            .map(|p| &p.link)
            .chain(current_plan.iter())
            .chain(geometries.iter())
            .chain(steady_flows.iter())
            .chain(unsteady_flows.iter())
            .chain(quasi_unsteady_flows.iter())
            .chain(
                plans
                    .iter()
                    .filter_map(|p| p.plan.as_ref().ok())
                    .flat_map(|p| p.geometry.iter().chain(p.flow.iter().map(|f| &f.link))),
            )
        {
            if let Some(error) = &link.diagnostic {
                diagnostics.push(ProjectDiagnostic::LinkError {
                    declared: link.declared.clone(),
                    error: error.clone(),
                });
            }
        }
        Ok(Self {
            source,
            title,
            program_version,
            units,
            current_plan,
            plans,
            geometries,
            steady_flows,
            unsteady_flows,
            quasi_unsteady_flows,
            diagnostics,
        })
    }
}

#[cfg(test)]
mod unicode_path_tests {
    use super::path_case_eq;
    use std::ffi::OsStr;
    #[test]
    fn ascii_case_policy_rejects_unicode_aliases() {
        assert!(path_case_eq(OsStr::new("Data"), OsStr::new("DATA")));
        for (a, b) in [
            ("i", "ı"),
            ("s", "ſ"),
            ("σ", "ς"),
            ("µ", "μ"),
            ("ა", "Ა"),
            ("ǆ", "ǅ"),
        ] {
            assert!(!path_case_eq(OsStr::new(a), OsStr::new(b)), "{a}/{b}");
        }
        assert!(!path_case_eq(OsStr::new("Straße"), OsStr::new("STRASSE")));
        assert!(!path_case_eq(OsStr::new("ä"), OsStr::new("a\u{308}")));
    }
}
