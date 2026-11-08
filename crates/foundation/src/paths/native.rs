//! Conversion between the Windows verbatim (extended-length) form and the
//! native form.
//!
//! Both directions matter. Stripping `\\?\` is required before a path is shown
//! or stored; re-adding it is required before a long path crosses a Win32
//! boundary. The application manifest is `longPathAware`, but `std::fs` does
//! not add the prefix on its own and OS calls still use the system form.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use super::resolved::PathError;

/// Longest native path that legacy Win32 entry points accept without the
/// verbatim prefix. `CreateDirectoryW` reserves twelve characters below
/// `MAX_PATH`, so the conservative bound applies to every operation.
#[cfg(windows)]
const WIN32_LEGACY_PATH_LIMIT: usize = 247;

/// Render a path in the form that may be displayed, logged, persisted, or sent
/// to a model.
///
/// On Windows this converts `\\?\C:\x` to `C:\x` and `\\?\UNC\server\share\x` to
/// `\\server\share\x`. A device namespace (`\\.\PhysicalDrive0`) or an opaque
/// verbatim namespace (`\\?\Volume{...}`) has no native spelling and is
/// rejected rather than silently mangled.
#[cfg(not(windows))]
pub fn native_form(path: &Path) -> Result<PathBuf, PathError> {
    Ok(path.to_path_buf())
}

#[cfg(windows)]
pub fn native_form(path: &Path) -> Result<PathBuf, PathError> {
    use std::ffi::OsString;
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Ok(path.to_path_buf());
    };
    let mut native = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => {
            let mut root = String::with_capacity(3);
            root.push(char::from(drive));
            root.push_str(":\\");
            PathBuf::from(root)
        }
        Prefix::VerbatimUNC(server, share) => {
            let mut root = OsString::from(r"\\");
            root.push(server);
            root.push(r"\");
            root.push(share);
            root.push(r"\");
            PathBuf::from(root)
        }
        Prefix::Verbatim(_) | Prefix::DeviceNS(_) => {
            return Err(PathError::UnsupportedNamespace(
                path.to_string_lossy().into_owned(),
            ));
        }
        Prefix::Disk(_) | Prefix::UNC(_, _) => return Ok(path.to_path_buf()),
    };
    for component in components {
        if !matches!(component, Component::RootDir) {
            native.push(component.as_os_str());
        }
    }
    Ok(native)
}

/// Render a native path in the form that may be handed to an operating system
/// call.
///
/// Everywhere except Windows this is the identity. On Windows a path that is
/// long enough to fall foul of the legacy limit is given the verbatim prefix,
/// which is only sound because the input is already absolute and free of `.`
/// and `..` — Win32 does not normalize a verbatim path.
#[cfg(not(windows))]
pub fn system_form(native: &Path) -> Cow<'_, Path> {
    Cow::Borrowed(native)
}

#[cfg(windows)]
pub fn system_form(native: &Path) -> Cow<'_, Path> {
    use std::ffi::OsString;
    use std::path::{Component, Prefix};

    if native.as_os_str().len() <= WIN32_LEGACY_PATH_LIMIT {
        return Cow::Borrowed(native);
    }
    // A verbatim path is passed to the object manager literally, so prefixing
    // anything that still carries `.`, `..`, or a relative root would produce a
    // string the OS cannot open. Leave those alone.
    if native
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Cow::Borrowed(native);
    }
    let Some(Component::Prefix(prefix)) = native.components().next() else {
        return Cow::Borrowed(native);
    };
    match prefix.kind() {
        Prefix::Disk(_) => {
            let mut system = OsString::from(r"\\?\");
            system.push(native.as_os_str());
            Cow::Owned(PathBuf::from(system))
        }
        Prefix::UNC(_, _) => {
            // `\\server\share\x` becomes `\\?\UNC\server\share\x`: drop the two
            // leading separators and splice in the UNC marker.
            let tail = native.as_os_str().to_string_lossy();
            let Some(tail) = tail.strip_prefix(r"\\") else {
                return Cow::Borrowed(native);
            };
            Cow::Owned(PathBuf::from(format!(r"\\?\UNC\{tail}")))
        }
        Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _) | Prefix::Verbatim(_) => {
            Cow::Borrowed(native)
        }
        Prefix::DeviceNS(_) => Cow::Borrowed(native),
    }
}
