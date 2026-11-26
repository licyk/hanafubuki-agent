//! Bounded, symlink-refusing filesystem primitives shared by the catalog.

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};

use crate::model::{SkillError, SkillResourceKind, MAX_SKILL_FILES};

/// How deep a Skill directory may nest.
pub(crate) const MAX_DIRECTORY_DEPTH: usize = 8;

static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_temporary_token() -> u64 {
    TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

/// Native text of a path, for people, logs, records, and error messages.
pub(crate) fn native_text(path: &Path) -> String {
    ResolvedPath::resolve(path)
        .map(|resolved| resolved.native_text().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Reject a path that escaped the directory it claims to be inside. Identity,
/// not text: the two can reach this check from different sources and on Windows
/// need not agree on case.
pub(crate) fn ensure_below(root: &Path, path: &Path) -> Result<(), SkillError> {
    if !PathKey::of(path).starts_with(&PathKey::of(root)) {
        return Err(SkillError::InvalidPath(format!(
            "{} is outside {}",
            native_text(path),
            native_text(root)
        )));
    }
    Ok(())
}

/// Normalize a package-relative path a caller supplied, refusing anything that
/// is absolute, escapes, or names a hidden entry.
pub(crate) fn normalize_relative(path: &str) -> Result<String, SkillError> {
    let invalid = || SkillError::InvalidPath(path.to_string());
    // An absolute path is refused outright rather than quietly reinterpreted as
    // a relative one: a caller that sent `/etc/passwd` meant something this
    // function must not silently rewrite into `etc/passwd`.
    if path.is_empty() || path.len() > 1024 || Path::new(path).is_absolute() {
        return Err(invalid());
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(invalid());
    }
    let mut segments = Vec::new();
    for segment in path.split(['/', '\\']) {
        if segment.is_empty() || segment == "." {
            continue;
        }
        let valid = segment != ".."
            && !segment.starts_with('.')
            && segment.len() <= 96
            && !segment.chars().any(|character| {
                character.is_control() || matches!(character, ':' | '<' | '>' | '"' | '|')
            });
        if !valid {
            return Err(invalid());
        }
        segments.push(segment);
    }
    if segments.is_empty() {
        return Err(invalid());
    }
    Ok(segments.join("/"))
}

/// The package-relative form of a path already known to live under `root`.
pub(crate) fn relative_from(root: &Path, path: &Path) -> Result<String, SkillError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| SkillError::InvalidPath(native_text(path)))?;
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::CurDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return Err(SkillError::InvalidPath(native_text(path)));
    }
    relative
        .iter()
        .map(|part| {
            part.to_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| SkillError::InvalidPath(native_text(path)))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}

/// One regular file discovered inside a Skill directory.
pub(crate) struct DiscoveredFile {
    pub(crate) relative_path: String,
    pub(crate) path: PathBuf,
    pub(crate) size: u64,
    pub(crate) kind: SkillResourceKind,
}

/// Files under `root` a model would read, sorted by relative path: the
/// `document` and everything a Skill ships beside it.
///
/// The document is always listed even when it is named in lowercase, so a
/// package copied here loads and reads back exactly as it did elsewhere.
/// Symlinks, hidden entries, and anything that is neither a file nor a
/// directory are skipped rather than fatal: a stray `.DS_Store` or an editor's
/// socket must not stop a package from loading. A package may carry much more
/// than it instructs — a wrapped research suite's fixtures, evaluation runs and
/// vendored sources — so what it lists is bounded by [`MAX_SKILL_FILES`] for
/// anything installed or duplicated through the catalog, and refused there
/// rather than silently truncated.
///
/// A file's size is recorded but never judged. Real packages ship multi-megabyte
/// templates and reference documents — `openai-templates` ships a `.pptx` larger
/// than any sane instruction budget — and refusing the whole Skill because of
/// one of them loses the Skill over a file the model was never going to read
/// inline. What may be *read* is bounded where the read happens.
pub(crate) fn collect_files(root: &Path) -> Result<Vec<DiscoveredFile>, SkillError> {
    let mut files = Vec::new();
    if let Some(manifest) = crate::discovery::manifest_path(root) {
        push_file(root, manifest, &mut files)?;
    }
    collect_into(root, root, 0, &mut files)?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files.dedup_by(|left, right| left.relative_path == right.relative_path);
    if files.len() > MAX_SKILL_FILES {
        // Discovery never applies this: what a package ships is the package's
        // business. A managed Skill is what an install or a duplication carries,
        // and that is where a listing this large stops being a Skill package.
        return Err(SkillError::LimitExceeded(format!(
            "a Skill to install or duplicate may hold at most {MAX_SKILL_FILES} files, found {}",
            files.len()
        )));
    }
    Ok(files)
}

/// The size of a regular file, or `None` for a directory, a symlink, or
/// anything that cannot be inspected.
fn regular_file_size(path: &Path) -> Option<u64> {
    let metadata = fs::symlink_metadata(path).ok()?;
    (!metadata.file_type().is_symlink() && metadata.is_file()).then_some(metadata.len())
}

/// Record one file.
fn push_file(
    root: &Path,
    path: PathBuf,
    output: &mut Vec<DiscoveredFile>,
) -> Result<(), SkillError> {
    let Some(size) = regular_file_size(&path) else {
        return Ok(());
    };
    let relative_path = relative_from(root, &path)?;
    output.push(DiscoveredFile {
        kind: crate::discovery::resource_kind(&relative_path),
        relative_path,
        path,
        size,
    });
    Ok(())
}

fn collect_into(
    root: &Path,
    directory: &Path,
    depth: usize,
    output: &mut Vec<DiscoveredFile>,
) -> Result<(), SkillError> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(SkillError::LimitExceeded(format!(
            "a Skill directory may nest at most {MAX_DIRECTORY_DEPTH} levels"
        )));
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return Ok(());
    };
    let mut entries = entries.collect::<Result<Vec<_>, _>>().unwrap_or_default();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_into(root, &path, depth.saturating_add(1), output)?;
        } else if metadata.is_file() {
            push_file(root, path, output)?;
        }
    }
    Ok(())
}

/// The first `maximum` bytes of a regular file, and whether it is longer.
pub(crate) fn read_bounded(path: &Path, maximum: u64) -> Result<(Vec<u8>, bool), SkillError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| SkillError::Io(format!("inspect {}: {error}", native_text(path))))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SkillError::InvalidPath(format!(
            "{} is not a regular file",
            native_text(path)
        )));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(maximum.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(|error| SkillError::Io(format!("read {}: {error}", native_text(path))))?;
    let truncated = bytes.len() as u64 > maximum;
    bytes.truncate(maximum as usize);
    Ok((bytes, truncated))
}

pub(crate) fn ensure_directory(path: &Path, label: &str) -> Result<(), SkillError> {
    fs::create_dir_all(path).map_err(|error| SkillError::Io(format!("create {label}: {error}")))?;
    if !path.is_dir() {
        return Err(SkillError::InvalidPath(format!(
            "{label} must be a directory"
        )));
    }
    Ok(())
}

/// The directory a managed root that is a link names.
///
/// A user root linked to another tool's skills directory — a shared `skills/`
/// folder, a checkout — is served as it is, the way that tool serves it. Only
/// the root itself is followed: the files inside are still the Skill's own.
pub(crate) fn link_target(path: &Path) -> Option<PathBuf> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_symlink() {
        return None;
    }
    ResolvedPath::resolve(path)
        .ok()
        .map(|resolved| resolved.native().to_path_buf())
        .filter(|target| target.is_dir())
}

pub(crate) fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), SkillError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| SkillError::Io(format!("create Skill directory: {error}")))?;
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| SkillError::Io(format!("create {}: {error}", native_text(path))))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| SkillError::Io(format!("write {}: {error}", native_text(path))))
}

/// Replace a file's contents through a sibling temporary, so a crash mid-write
/// never leaves a half-written document behind.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8], label: &str) -> Result<(), SkillError> {
    let parent = path
        .parent()
        .ok_or_else(|| SkillError::InvalidPath(format!("{label} has no parent")))?;
    fs::create_dir_all(parent)
        .map_err(|error| SkillError::Io(format!("create {label} directory: {error}")))?;
    if path.exists() {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| SkillError::Io(format!("inspect {label}: {error}")))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SkillError::InvalidPath(format!(
                "{label} must be a regular non-symlink file"
            )));
        }
    }
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("skill"),
        std::process::id(),
        next_temporary_token(),
    ));
    write_new_file(&temporary, bytes)?;
    publish_file(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        SkillError::Io(format!("publish {label}: {error}"))
    })?;
    sync_directory(parent)
}

/// Copy what a Skill ships, exactly as the catalog lists it: the document, the
/// interface file, and the package's files, with symlinks and hidden entries
/// skipped so a copy never follows one out of the package.
pub(crate) fn copy_tree(source: &Path, destination: &Path) -> Result<(), SkillError> {
    ensure_directory(destination, "Skill directory")?;
    for file in collect_files(source)? {
        let target = destination.join(&file.relative_path);
        ensure_below(destination, &target)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| SkillError::Io(format!("create Skill directory: {error}")))?;
        }
        fs::copy(&file.path, &target)
            .map_err(|error| SkillError::Io(format!("copy {}: {error}", file.relative_path)))?;
    }
    Ok(())
}

/// Publish `staging` at `destination`, replacing whatever is there. The old
/// directory is moved aside first and removed only once the new one is in
/// place, so a failure leaves the previous Skill intact.
pub(crate) fn replace_directory(staging: &Path, destination: &Path) -> Result<(), SkillError> {
    let parent = destination
        .parent()
        .ok_or_else(|| SkillError::InvalidPath("Skill destination has no parent".to_string()))?;
    if !destination.exists() {
        return fs::rename(staging, destination)
            .map_err(|error| SkillError::Io(format!("publish Skill: {error}")))
            .and_then(|()| sync_directory(parent));
    }
    let backup = parent.join(format!(
        ".{}.replaced.{}.{}",
        destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("skill"),
        std::process::id(),
        next_temporary_token(),
    ));
    fs::rename(destination, &backup)
        .map_err(|error| SkillError::Io(format!("reserve the replaced Skill: {error}")))?;
    if let Err(error) = fs::rename(staging, destination) {
        let _ = fs::rename(&backup, destination);
        return Err(SkillError::Io(format!("publish Skill: {error}")));
    }
    let _ = fs::remove_dir_all(&backup);
    sync_directory(parent)
}

#[cfg(not(windows))]
fn publish_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn publish_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> Result<(), SkillError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| SkillError::Io(format!("sync {}: {error}", native_text(path))))
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(_path: &Path) -> Result<(), SkillError> {
    Ok(())
}

#[cfg(test)]
mod tests;
