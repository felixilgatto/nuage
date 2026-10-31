use serde::{Deserialize, Serialize};

/// Result type of the Nuage crates.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error returned by the engine and sent to clients over IPC.
///
/// Messages are meant to be understood by non-technical users and should say
/// what to do next. The variant is the stable machine-readable code
/// (`"code"` in JSON).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "code", content = "message", rename_all = "snake_case")]
pub enum Error {
    #[error("Not implemented yet: {0}")]
    NotImplemented(String),
    #[error("{0}")]
    InvalidArgument(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    AlreadyExists(String),
    #[error("{0}")]
    AuthRequired(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Busy(String),
    #[error("rclone: {0}")]
    Rclone(String),
    #[error("{0}")]
    Io(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl Error {
    pub fn not_implemented(what: impl Into<String>) -> Self {
        Self::NotImplemented(what.into())
    }

    /// Stable machine-readable code, as serialised in JSON.
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotImplemented(_) => "not_implemented",
            Error::InvalidArgument(_) => "invalid_argument",
            Error::NotFound(_) => "not_found",
            Error::AlreadyExists(_) => "already_exists",
            Error::AuthRequired(_) => "auth_required",
            Error::Unsupported(_) => "unsupported",
            Error::Busy(_) => "busy",
            Error::Rclone(_) => "rclone",
            Error::Io(_) => "io",
            Error::Internal(_) => "internal",
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_shape_matches_code() {
        let e = Error::not_implemented("mount");
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["code"], e.code());
        assert_eq!(json["message"], "mount");
        assert_eq!(e.to_string(), "Not implemented yet: mount");
    }
}
