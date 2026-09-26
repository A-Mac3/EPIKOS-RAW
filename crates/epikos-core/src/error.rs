use std::fmt;

/// Recoverable engine failures. Decode and I/O errors wrap the underlying cause.
#[derive(Debug)]
pub enum Error {
    UnsupportedFormat { path: String, detail: String },
    Decode(String),
    InvalidImage { reason: String },
    Sidecar(String),
    Io(std::io::Error),
    /// A newer request of the same kind replaced this one before it finished.
    Superseded,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnsupportedFormat { path, detail } => {
                write!(f, "unsupported camera file `{path}`: {detail}")
            }
            Error::Decode(msg) => write!(f, "decode failed: {msg}"),
            Error::InvalidImage { reason } => write!(f, "invalid image: {reason}"),
            Error::Sidecar(msg) => write!(f, "sidecar: {msg}"),
            Error::Io(err) => write!(f, "i/o: {err}"),
            Error::Superseded => write!(f, "superseded by a newer request"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Io(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
