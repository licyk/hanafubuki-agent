//! [`ResolvedPath`] — a filesystem location resolved exactly once, carrying
//! every form the rest of the application is permitted to use.

use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use super::key::PathKey;
use super::native::{native_form, system_form};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathError {
    Empty,
    /// The location does not exist. Kept distinct from every other resolution
    /// failure so a caller can retry as a create without also swallowing
    /// permission errors or disconnected drives.
    NotFound(String),
    NotUtf8(String),
    UnsupportedNamespace(String),
    InvalidComponent(String),
    Io(String),
}

impl PathError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("path is empty"),
            Self::NotFound(path) => write!(formatter, "path does not exist: {path}"),
            Self::NotUtf8(path) => write!(formatter, "path is not valid UTF-8: {path}"),
            Self::UnsupportedNamespace(path) => write!(
                formatter,
                "unsupported Windows device or verbatim path namespace: {path}"
            ),
            Self::InvalidComponent(name) => {
                write!(formatter, "not a single path component: {name}")
            }
            Self::Io(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for PathError {}

/// A filesystem location resolved exactly once.
///
/// Constructing one is the only sanctioned way to call `fs::canonicalize`. The
/// three accessors are not interchangeable:
///
/// * [`Self::system`] for operating system calls and a child process's working
///   directory;
/// * [`Self::native`] / [`Self::native_text`] for anything a person or a model
///   reads, and for anything written to a durable record;
/// * [`Self::key`] for every comparison, prefix test, hash, and digest.
#[derive(Clone, Debug)]
pub struct ResolvedPath {
    system: PathBuf,
    native: PathBuf,
    native_text: String,
    key: PathKey,
}

impl ResolvedPath {
    /// Resolve an existing location.
    ///
    /// `.` and `..` are collapsed lexically *before* the filesystem is touched,
    /// because Win32 does not normalize a verbatim path and would otherwise
    /// fail to open a perfectly ordinary relative request.
    pub fn resolve(path: impl AsRef<Path>) -> Result<Self, PathError> {
        let requested = super::lexical_absolute(path.as_ref())?;
        let canonical =
            std::fs::canonicalize(system_form(&requested).as_ref()).map_err(|error| {
                let requested = requested.display().to_string();
                if error.kind() == std::io::ErrorKind::NotFound {
                    PathError::NotFound(requested)
                } else {
                    PathError::Io(format!("resolve {requested}: {error}"))
                }
            })?;
        Self::from_canonical(canonical)
    }

    /// Resolve a child of an already-resolved directory that need not exist.
    ///
    /// The parent has been resolved, so only the final component is new; it is
    /// checked to be a single ordinary name rather than a smuggled traversal.
    pub fn resolve_child(parent: &Self, file_name: &OsStr) -> Result<Self, PathError> {
        let mut components = Path::new(file_name).components();
        let (Some(Component::Normal(name)), None) = (components.next(), components.next()) else {
            return Err(PathError::InvalidComponent(
                file_name.to_string_lossy().into_owned(),
            ));
        };
        let native = parent.native.join(name);
        let native_text = utf8_text(&native)?;
        Ok(Self {
            system: parent.system.join(name),
            key: PathKey::of(&native),
            native,
            native_text,
        })
    }

    /// Absolutize a requested path against this location and collapse `.`/`..`.
    ///
    /// Returns the native form, ready to be handed to [`Self::resolve`]. An
    /// absolute request is normalized and returned as-is; a relative one is
    /// joined onto this location first.
    pub fn join_request(&self, requested: &Path) -> Result<PathBuf, PathError> {
        if requested.as_os_str().is_empty() {
            return Err(PathError::Empty);
        }
        let joined = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.native.join(requested)
        };
        native_form(&super::lexical_absolute(&joined)?)
    }

    /// Build from a path the caller has already canonicalized.
    ///
    /// Only for the boundary where an OS API hands back a final path; ordinary
    /// callers use [`Self::resolve`].
    pub fn from_canonical(canonical: PathBuf) -> Result<Self, PathError> {
        let native = native_form(&canonical)?;
        let native_text = utf8_text(&native)?;
        Ok(Self {
            key: PathKey::of(&native),
            system: canonical,
            native,
            native_text,
        })
    }

    /// The form the operating system resolved. On Windows this keeps the
    /// verbatim prefix and with it the ability to address long paths.
    pub fn system(&self) -> &Path {
        &self.system
    }

    /// The form a person or a model can read and type back.
    pub fn native(&self) -> &Path {
        &self.native
    }

    /// The form a person or a model can read and type back, as UTF-8.
    pub fn native_text(&self) -> &str {
        &self.native_text
    }

    /// The comparison identity. Never render this.
    pub fn key(&self) -> &PathKey {
        &self.key
    }

    /// Whether this location is `base` or lies beneath it.
    pub fn starts_with(&self, base: &Self) -> bool {
        self.key.starts_with(&base.key)
    }

    pub fn file_name(&self) -> Option<&OsStr> {
        self.native.file_name()
    }

    /// Re-resolve and report whether the location still refers to the same
    /// place. Cheaper than handing out the raw path for a caller to compare.
    pub fn still_resolves_here(&self) -> bool {
        Self::resolve(&self.native).is_ok_and(|current| current.key == self.key)
    }
}

impl PartialEq for ResolvedPath {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for ResolvedPath {}

fn utf8_text(path: &Path) -> Result<String, PathError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| PathError::NotUtf8(path.to_string_lossy().into_owned()))
}
