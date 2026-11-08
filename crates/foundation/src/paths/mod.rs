//! The single authority for filesystem path resolution, comparison, and
//! rendering.
//!
//! Every filesystem location plays three roles that must not share one value:
//!
//! * the **system form** handed to the operating system, which on Windows is the
//!   verbatim (`\\?\`) form returned by `GetFinalPathNameByHandleW` and which
//!   must be kept so long paths keep working;
//! * the **identity form** used for comparison, prefix tests, hashing, and
//!   approval digests, which must fold Windows prefix kinds and letter case;
//! * the **interchange form** shown to the user, written to records and logs,
//!   and sent to a model, which must be the native form a person can type back.
//!
//! Collapsing those three into one `PathBuf` is what leaks `\\?\E:\...` into
//! model transcripts and breaks case-insensitive containment checks, so this
//! module hands out [`ResolvedPath`], which carries all three explicitly, and
//! owns the only `fs::canonicalize` call in the workspace.

use std::path::{Path, PathBuf};

mod key;
mod native;
mod resolved;

pub use key::PathKey;
pub use native::{native_form, system_form};
pub use resolved::{PathError, ResolvedPath};

/// Lexically normalize a path and render it in the native form.
///
/// This performs no filesystem access: it absolutizes against the current
/// directory, collapses `.` and `..`, and converts a Windows verbatim prefix
/// back to its native spelling. Use it for a path that is being *composed*
/// rather than resolved; anything that must exist on disk goes through
/// [`ResolvedPath::resolve`] instead.
pub fn normalize_path(path: PathBuf) -> Result<PathBuf, String> {
    lexical_absolute(&path)
        .and_then(|path| native_form(&path))
        .map_err(|error| error.to_string())
}

/// The part of `path` below `base`, or `None` when `path` is neither `base`
/// nor beneath it.
///
/// Containment is decided by [`PathKey`], and the tail is rebuilt from
/// `path`'s own components rather than by trimming text, because the two may
/// agree on identity while differing in prefix kind or, on Windows, in case.
pub fn relative_below(path: &Path, base: &Path) -> Option<PathBuf> {
    let base_key = PathKey::of(base);
    if !PathKey::of(path).starts_with(&base_key) {
        return None;
    }
    Some(
        path.components()
            .skip(base_key.len())
            .map(|component| component.as_os_str())
            .collect(),
    )
}

pub fn validate_not_dangerous(path: &Path) -> Result<(), String> {
    if path.parent().is_none() {
        return Err("refusing to use filesystem root as a managed path".to_string());
    }
    Ok(())
}

/// Absolutize against the current directory and collapse `.`/`..` lexically.
///
/// The collapse never pops past the root: `C:\..\..\x` is `C:\x`, not an
/// attempt to escape the volume.
pub(crate) fn lexical_absolute(path: &Path) -> Result<PathBuf, PathError> {
    if path.as_os_str().is_empty() {
        return Err(PathError::Empty);
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| PathError::Io(format!("resolve current dir: {error}")))?
            .join(path)
    };
    Ok(collapse(&absolute))
}

fn collapse(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if depth > 0 {
                    out.pop();
                    depth -= 1;
                }
            }
            Component::Normal(value) => {
                out.push(value);
                depth += 1;
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests;
