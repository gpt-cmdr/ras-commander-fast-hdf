//! Stable error categories; messages retain source detail without controlling policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    Io,
    FileNotFound,
    Internal,
    NotFound,
    Unsupported,
    Corrupt,
    InvalidRequest,
    StructuralTopology,
    InvalidGeometry,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub path: Option<std::path::PathBuf>,
    pub subject: Option<String>,
    /// Why an `Unsupported` (or budget-limited `InvalidRequest`) error is unsupported.
    /// Carried through every conversion to a [`crate::Capability`]; never defaulted by
    /// the producing module.
    pub code: Option<crate::UnsupportedCode>,
}
impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            path: None,
            subject: None,
            code: None,
        }
    }
    /// An `Unsupported` error that carries its machine-readable reason.
    pub fn unsupported(code: crate::UnsupportedCode, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message).with_code(code)
    }
}
impl Error {
    pub fn with_code(mut self, code: crate::UnsupportedCode) -> Self {
        self.code = Some(code);
        self
    }
    pub fn with_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}
impl std::error::Error for Error {}
impl ErrorKind {
    /// Stable snake_case spelling (never the Debug form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Io => "io",
            Self::FileNotFound => "file_not_found",
            Self::Internal => "internal",
            Self::NotFound => "not_found",
            Self::Unsupported => "unsupported",
            Self::Corrupt => "corrupt",
            Self::InvalidRequest => "invalid_request",
            Self::StructuralTopology => "structural_topology",
            Self::InvalidGeometry => "invalid_geometry",
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(feature = "catalog")]
impl From<crate::catalog::CatalogError> for Error {
    fn from(e: crate::catalog::CatalogError) -> Self {
        use crate::catalog::CatalogErrorKind as K;
        let kind = match e.kind {
            K::NotFound => ErrorKind::FileNotFound,
            K::Io => ErrorKind::Io,
            K::Malformed => ErrorKind::Corrupt,
            K::UnsupportedEncoding => ErrorKind::Unsupported,
            K::AmbiguousPath => ErrorKind::InvalidRequest,
        };
        let err = Self::new(kind, e.message).with_path(e.path);
        if matches!(e.kind, K::UnsupportedEncoding) {
            err.with_code(crate::UnsupportedCode::Encoding)
        } else {
            err
        }
    }
}
