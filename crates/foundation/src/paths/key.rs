//! Path identity: the only value that may be compared, ordered, or hashed.
//!
//! A `PathBuf` is the wrong comparison type on Windows twice over. `\\?\E:\x`
//! and `E:\x` are the same location but differ in prefix kind, and `E:\Models`
//! and `e:\models` are the same location but differ in case. `Path::starts_with`
//! sees neither, so a containment or protection check written against raw paths
//! silently fails open.

use std::ffi::OsStr;
use std::path::{Component, Path, Prefix};

/// Case-folded on Windows, byte-exact everywhere else.
#[cfg(windows)]
type KeyText = Vec<u16>;
#[cfg(not(windows))]
type KeyText = std::ffi::OsString;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum PathKeyComponent {
    Disk(u8),
    Unc(KeyText, KeyText),
    Namespace(KeyText),
    Root,
    Current,
    Parent,
    Normal(KeyText),
}

/// The comparison identity of a filesystem location.
///
/// Deliberately not renderable: a key is for deciding whether two paths are the
/// same place, never for telling anyone where that place is. Use
/// [`super::ResolvedPath::native_text`] for that.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathKey(Vec<PathKeyComponent>);

impl PathKey {
    pub fn of(path: &Path) -> Self {
        Self(
            path.components()
                .map(|component| match component {
                    Component::Prefix(prefix) => match prefix.kind() {
                        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                            PathKeyComponent::Disk(drive.to_ascii_lowercase())
                        }
                        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                            PathKeyComponent::Unc(key_text(server), key_text(share))
                        }
                        Prefix::Verbatim(namespace) | Prefix::DeviceNS(namespace) => {
                            PathKeyComponent::Namespace(key_text(namespace))
                        }
                    },
                    Component::RootDir => PathKeyComponent::Root,
                    Component::CurDir => PathKeyComponent::Current,
                    Component::ParentDir => PathKeyComponent::Parent,
                    Component::Normal(value) => PathKeyComponent::Normal(key_text(value)),
                })
                .collect(),
        )
    }

    /// Whether this location is `base` or lies beneath it.
    ///
    /// Component-wise, so `C:\models-extra` does not start with `C:\models`.
    pub fn starts_with(&self, base: &PathKey) -> bool {
        self.0.starts_with(&base.0)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of path components, so a caller can strip a known base without
    /// re-deriving the relative form from raw strings.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// A stable byte encoding for digests and approval records.
    ///
    /// Digesting the rendered path instead would tie an approval to a spelling:
    /// a case-only rename on Windows, or the same location reached by a
    /// different prefix kind, would invalidate a grant that still refers to
    /// exactly the same file. Components are length-prefixed so no two
    /// component sequences can encode to the same bytes.
    pub fn digest_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for component in &self.0 {
            match component {
                PathKeyComponent::Disk(drive) => {
                    out.push(b'd');
                    out.push(*drive);
                }
                PathKeyComponent::Unc(server, share) => {
                    out.push(b'u');
                    push_text(&mut out, server);
                    push_text(&mut out, share);
                }
                PathKeyComponent::Namespace(value) => {
                    out.push(b'x');
                    push_text(&mut out, value);
                }
                PathKeyComponent::Root => out.push(b'/'),
                PathKeyComponent::Current => out.push(b'.'),
                PathKeyComponent::Parent => out.push(b'^'),
                PathKeyComponent::Normal(value) => {
                    out.push(b'n');
                    push_text(&mut out, value);
                }
            }
        }
        out
    }
}

#[cfg(windows)]
fn push_text(out: &mut Vec<u8>, text: &KeyText) {
    out.extend_from_slice(&(text.len() as u64).to_le_bytes());
    for unit in text {
        out.extend_from_slice(&unit.to_le_bytes());
    }
}

#[cfg(not(windows))]
fn push_text(out: &mut Vec<u8>, text: &KeyText) {
    use std::os::unix::ffi::OsStrExt;

    let bytes = text.as_os_str().as_bytes();
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

#[cfg(windows)]
fn key_text(value: &OsStr) -> KeyText {
    use std::os::windows::ffi::OsStrExt;

    value
        .encode_wide()
        .map(|unit| {
            if unit >= u16::from(b'A') && unit <= u16::from(b'Z') {
                unit + u16::from(b'a' - b'A')
            } else {
                unit
            }
        })
        .collect()
}

#[cfg(not(windows))]
fn key_text(value: &OsStr) -> KeyText {
    value.to_os_string()
}
