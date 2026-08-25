use std::path::PathBuf;

/// Everything that can go wrong while reading a BioWare container or resource.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("{path}: not a {expected} file (found signature {found:?})")]
    BadSignature {
        path: PathBuf,
        expected: &'static str,
        found: String,
    },

    #[error("{path}: unsupported {format} version {version:?}")]
    BadVersion {
        path: PathBuf,
        format: &'static str,
        version: String,
    },

    #[error("{path}: truncated at byte {offset} (needed {needed} more bytes, file is {len})")]
    Truncated {
        path: PathBuf,
        offset: usize,
        needed: usize,
        len: usize,
    },

    #[error("{path}: {message}")]
    Malformed { path: PathBuf, message: String },

    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, FormatError>;
