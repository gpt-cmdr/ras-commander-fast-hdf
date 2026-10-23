#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnsupportedCode {
    Encoding,
    Schema,
    IdentityLayout,
    ResourceLimit,
    NotImplemented,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Capability {
    Supported,
    Unavailable {
        reason: String,
    },
    Unsupported {
        code: UnsupportedCode,
        reason: String,
    },
    Corrupt {
        reason: String,
    },
    Delegated {
        reason: String,
    },
}
impl UnsupportedCode {
    /// Stable snake_case spelling (never the Debug form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Encoding => "encoding",
            Self::Schema => "schema",
            Self::IdentityLayout => "identity_layout",
            Self::ResourceLimit => "resource_limit",
            Self::NotImplemented => "not_implemented",
        }
    }
}
impl Capability {
    pub fn unsupported(code: UnsupportedCode, reason: impl Into<String>) -> Self {
        Self::Unsupported {
            code,
            reason: reason.into(),
        }
    }
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }
    pub fn corrupt(reason: impl Into<String>) -> Self {
        Self::Corrupt {
            reason: reason.into(),
        }
    }
    pub fn delegated(reason: impl Into<String>) -> Self {
        Self::Delegated {
            reason: reason.into(),
        }
    }
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Supported => None,
            Self::Unavailable { reason }
            | Self::Unsupported { reason, .. }
            | Self::Corrupt { reason }
            | Self::Delegated { reason } => Some(reason),
        }
    }
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }
}
#[cfg(feature = "hdf")]
/// The one crate-private mapping from a failed check to a family capability.
/// `Io`, `Internal` and `FileNotFound` always propagate. `Unsupported` keeps the error's code
/// (`Schema` only when a producer forgot to set one). Only an `InvalidRequest` that already
/// carries `ResourceLimit` (a byte budget) becomes `Unsupported(resource_limit)`; every other
/// `InvalidRequest` (type mismatch, slice bounds, caller errors) propagates as `Err`.
pub(crate) fn error_to_capability(err: crate::Error) -> crate::Result<Capability> {
    use crate::ErrorKind as K;
    Ok(match err.kind {
        K::Io | K::Internal | K::FileNotFound => return Err(err),
        // Canonical absence is decided by the caller through `absent_ok` before mapping;
        // a NotFound that reaches this point was promised present, so it is corruption.
        K::NotFound => Capability::corrupt(err.message),
        K::Unsupported => {
            Capability::unsupported(err.code.unwrap_or(UnsupportedCode::Schema), err.message)
        }
        K::InvalidRequest if err.code == Some(UnsupportedCode::ResourceLimit) => {
            Capability::unsupported(UnsupportedCode::ResourceLimit, err.message)
        }
        K::InvalidRequest => return Err(err),
        K::Corrupt | K::StructuralTopology | K::InvalidGeometry => Capability::corrupt(err.message),
    })
}
#[cfg(feature = "hdf")]
/// Combine several failures of one family: propagating kinds win, then any corruption, then
/// the first unsupported code. Reasons are the bare messages joined with `; `.
pub(crate) fn errors_to_capability(errors: Vec<crate::Error>) -> crate::Result<Option<Capability>> {
    let mut caps = Vec::new();
    for e in errors {
        caps.push(error_to_capability(e)?);
    }
    if caps.is_empty() {
        return Ok(None);
    }
    let reasons = caps
        .iter()
        .filter_map(|c| c.reason())
        .collect::<Vec<_>>()
        .join("; ");
    Ok(Some(
        if caps.iter().any(|c| matches!(c, Capability::Corrupt { .. })) {
            Capability::corrupt(reasons)
        } else if let Some(code) = caps.iter().find_map(|c| match c {
            Capability::Unsupported { code, .. } => Some(*code),
            _ => None,
        }) {
            Capability::unsupported(code, reasons)
        } else {
            Capability::unavailable(reasons)
        },
    ))
}
#[cfg(feature = "hdf")]
/// Canonical absence: `NotFound` becomes `None`; every other failure propagates unchanged.
pub(crate) fn absent_ok<T>(result: crate::Result<T>) -> crate::Result<Option<T>> {
    match result {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.kind == crate::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

impl Capability {
    /// The unsupported code, when this is `Unsupported`.
    pub fn code(&self) -> Option<UnsupportedCode> {
        match self {
            Self::Unsupported { code, .. } => Some(*code),
            _ => None,
        }
    }
    /// Stable snake_case spelling of the variant (never a Debug form).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unavailable { .. } => "unavailable",
            Self::Unsupported { .. } => "unsupported",
            Self::Corrupt { .. } => "corrupt",
            Self::Delegated { .. } => "delegated",
        }
    }
}

/// One reported stored value with a single status. `capability` is the only status:
/// `Supported` if and only if `value` is present. Unavailable means the object carries no such
/// value; Unsupported and Corrupt keep the exact stored `raw_bytes` when they exist. Fields are
/// private so no contradictory state (a value beside a failure, or a failure that is
/// `Supported`) can be constructed.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Reported<T> {
    value: Option<T>,
    raw_bytes: Option<Vec<u8>>,
    capability: Capability,
}
impl<T> Reported<T> {
    /// A proven value (status Supported).
    pub fn supported(value: T) -> Self {
        Self {
            value: Some(value),
            raw_bytes: None,
            capability: Capability::Supported,
        }
    }
    /// Nothing is stored for this item (status Unavailable).
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::failed(Capability::unavailable(reason.into()))
    }
    /// Stored but not provable as the requested type (status Unsupported with its code).
    pub fn unsupported(code: UnsupportedCode, reason: impl Into<String>) -> Self {
        Self::failed(Capability::unsupported(code, reason.into()))
    }
    /// Stored but damaged or contradictory (status Corrupt).
    pub fn corrupt(reason: impl Into<String>) -> Self {
        Self::failed(Capability::corrupt(reason.into()))
    }
    fn failed(capability: Capability) -> Self {
        Self {
            value: None,
            raw_bytes: None,
            capability,
        }
    }
    /// Attach the exact stored payload. An Unavailable item stores nothing, so this is a no-op
    /// there and `raw_bytes()` stays `None`.
    pub fn with_raw_bytes(mut self, raw_bytes: Vec<u8>) -> Self {
        if !self.is_unavailable() {
            self.raw_bytes = Some(raw_bytes);
        }
        self
    }
    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }
    pub fn into_value(self) -> Option<T> {
        self.value
    }
    /// Exact stored payload, when one exists.
    pub fn raw_bytes(&self) -> Option<&[u8]> {
        self.raw_bytes.as_deref()
    }
    /// The single status of this item.
    pub fn capability(&self) -> &Capability {
        &self.capability
    }
    pub fn is_supported(&self) -> bool {
        self.capability.is_supported()
    }
    /// True when nothing is stored (status Unavailable).
    pub fn is_unavailable(&self) -> bool {
        matches!(self.capability, Capability::Unavailable { .. })
    }
}

#[cfg(all(test, feature = "hdf"))]
mod tests {
    use super::*;
    use crate::{Error, ErrorKind};

    #[test]
    fn propagating_kinds_never_become_statuses() {
        for kind in [ErrorKind::Io, ErrorKind::Internal, ErrorKind::FileNotFound] {
            let e = error_to_capability(Error::new(kind, "x")).unwrap_err();
            assert_eq!(e.kind, kind);
            assert!(errors_to_capability(vec![Error::new(ErrorKind::Corrupt, "c"), e]).is_err());
        }
    }

    #[test]
    fn codes_and_kinds_map_to_one_status_each() {
        let cap = |e| error_to_capability(e).unwrap();
        assert_eq!(
            cap(Error::unsupported(UnsupportedCode::IdentityLayout, "l")).code(),
            Some(UnsupportedCode::IdentityLayout)
        );
        assert_eq!(
            cap(Error::new(ErrorKind::InvalidRequest, "b")
                .with_code(UnsupportedCode::ResourceLimit))
            .code(),
            Some(UnsupportedCode::ResourceLimit)
        );
        // Any other InvalidRequest is a caller error and propagates.
        let e = error_to_capability(Error::new(ErrorKind::InvalidRequest, "slice")).unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
        assert_eq!(
            cap(Error::new(ErrorKind::Unsupported, "forgotten code")).code(),
            Some(UnsupportedCode::Schema)
        );
        for kind in [
            ErrorKind::Corrupt,
            ErrorKind::NotFound,
            ErrorKind::StructuralTopology,
            ErrorKind::InvalidGeometry,
        ] {
            assert!(matches!(
                cap(Error::new(kind, "d")),
                Capability::Corrupt { .. }
            ));
        }
    }

    #[test]
    fn combined_failures_prefer_corrupt_then_first_code() {
        let merged = errors_to_capability(vec![
            Error::unsupported(UnsupportedCode::Encoding, "a"),
            Error::unsupported(UnsupportedCode::ResourceLimit, "b"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(merged.code(), Some(UnsupportedCode::Encoding));
        assert_eq!(merged.reason(), Some("a; b"));
        let with_corrupt = errors_to_capability(vec![
            Error::unsupported(UnsupportedCode::Encoding, "a"),
            Error::new(ErrorKind::Corrupt, "b"),
        ])
        .unwrap()
        .unwrap();
        assert!(matches!(with_corrupt, Capability::Corrupt { .. }));
        assert_eq!(errors_to_capability(Vec::new()).unwrap(), None);
    }

    #[test]
    fn absence_is_only_not_found() {
        assert_eq!(
            absent_ok::<u8>(Err(Error::new(ErrorKind::NotFound, "n"))).unwrap(),
            None
        );
        assert_eq!(absent_ok(Ok(3u8)).unwrap(), Some(3));
        assert!(absent_ok::<u8>(Err(Error::new(ErrorKind::Corrupt, "c"))).is_err());
    }
}
