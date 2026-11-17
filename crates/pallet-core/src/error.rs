use std::path::PathBuf;

/// Every fallible operation in pallet-core returns this error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{binary} is not installed or not on PATH")]
    BinaryMissing { binary: String },

    #[error("command failed (exit {code}): {command}\n{output}")]
    CommandFailed {
        command: String,
        code: i32,
        output: String,
    },

    #[error("command timed out after {seconds}s: {command}")]
    CommandTimeout { command: String, seconds: u64 },

    #[error("invalid package name: {0}")]
    InvalidName(String),

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("unsafe archive entry in {path}: {reason}")]
    UnsafeArchive { path: PathBuf, reason: String },

    #[error("checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    #[error("signature verification failed for {path}: {reason}")]
    SignatureInvalid { path: PathBuf, reason: String },

    #[error("no supported source could handle {0}")]
    UnsupportedInstallTarget(String),

    #[error("source '{0}' is not available on this system")]
    SourceUnavailable(String),

    #[error("package '{0}' was not found")]
    NotFound(String),

    #[error("operation requires confirmation: {0}")]
    ConfirmationRequired(String),

    #[error("no privilege escalation helper (pkexec/sudo) was found")]
    NoPrivilegeHelper,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Toml(#[from] toml::de::Error),

    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
