use std::fmt;

/// Why a store operation failed; the message is the whole of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageError {
    /// The request, a manifest or a marketplace cannot be used.
    Invalid(String),
    /// A package, a marketplace or one of its plugins is not there.
    NotFound(String),
    /// A file could not be read, written, copied or removed.
    Io(String),
    /// `git` failed or could not run.
    Git(String),
}

impl fmt::Display for PackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message)
            | Self::NotFound(message)
            | Self::Io(message)
            | Self::Git(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for PackageError {}
