use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hanafubuki_agent_foundation::cancellation::CancellationToken;

use crate::resolver::sha256_hex;
use crate::WorkspaceFsError;

const WRITE_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy)]
pub(crate) enum AtomicPublishMode {
    CreateNew,
    ReplaceExisting,
}

pub(crate) trait AtomicPublishHook: Send + Sync {
    fn after_write_chunk(&self, _temporary: &Path, _written_bytes: usize) -> Result<(), String> {
        Ok(())
    }

    fn before_publish(&self, temporary: &Path, destination: &Path) -> Result<(), String>;
}

#[derive(Default)]
pub(crate) struct NoopAtomicPublishHook;

impl AtomicPublishHook for NoopAtomicPublishHook {
    fn before_publish(&self, _temporary: &Path, _destination: &Path) -> Result<(), String> {
        Ok(())
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "independent inputs that a parameter struct would only regroup"
)]
pub(crate) fn atomic_write(
    destination: &Path,
    bytes: &[u8],
    expected_sha256: &str,
    existing_permissions: Option<fs::Permissions>,
    mode: AtomicPublishMode,
    cancellation: &CancellationToken,
    hook: &Arc<dyn AtomicPublishHook>,
    revalidate: impl FnOnce() -> Result<(), WorkspaceFsError>,
) -> Result<(), WorkspaceFsError> {
    if cancellation.is_cancelled() {
        return Err(WorkspaceFsError::Cancelled);
    }
    let parent = destination.parent().ok_or_else(|| {
        WorkspaceFsError::Resolution("write destination has no parent".to_string())
    })?;
    let (temporary, mut file) = create_temporary(parent)?;
    let guard = TemporaryGuard(temporary.clone());
    let mut written_bytes = 0usize;
    for chunk in bytes.chunks(WRITE_CHUNK_BYTES) {
        if cancellation.is_cancelled() {
            return Err(WorkspaceFsError::Cancelled);
        }
        file.write_all(chunk)
            .map_err(|error| WorkspaceFsError::Io(format!("write temporary file: {error}")))?;
        written_bytes = written_bytes.saturating_add(chunk.len());
        hook.after_write_chunk(&temporary, written_bytes)
            .map_err(WorkspaceFsError::Io)?;
    }
    file.flush()
        .map_err(|error| WorkspaceFsError::Io(format!("flush temporary file: {error}")))?;
    file.sync_all()
        .map_err(|error| WorkspaceFsError::Io(format!("sync temporary file: {error}")))?;
    if let Some(permissions) = existing_permissions {
        fs::set_permissions(&temporary, permissions)
            .map_err(|error| WorkspaceFsError::Io(format!("set temporary permissions: {error}")))?;
    }
    drop(file);
    let mut verified = Vec::with_capacity(bytes.len());
    File::open(&temporary)
        .and_then(|mut file| file.read_to_end(&mut verified))
        .map_err(|error| WorkspaceFsError::Io(format!("verify temporary file: {error}")))?;
    if verified.len() != bytes.len() || sha256_hex(&verified) != expected_sha256 {
        return Err(WorkspaceFsError::Io(
            "temporary file verification failed".to_string(),
        ));
    }
    if cancellation.is_cancelled() {
        return Err(WorkspaceFsError::Cancelled);
    }
    revalidate()?;
    hook.before_publish(&temporary, destination)
        .map_err(WorkspaceFsError::Io)?;
    if cancellation.is_cancelled() {
        return Err(WorkspaceFsError::Cancelled);
    }
    publish_file(&temporary, destination, mode)?;
    guard.disarm();
    sync_parent(parent)?;
    Ok(())
}

fn create_temporary(parent: &Path) -> Result<(PathBuf, File), WorkspaceFsError> {
    for _ in 0..32 {
        let mut random = [0u8; 12];
        getrandom::fill(&mut random)
            .map_err(|error| WorkspaceFsError::Io(format!("temporary name entropy: {error}")))?;
        let name = format!(
            ".hanafubuki-agent-write-{}",
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let path = parent.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(WorkspaceFsError::Io(format!(
                    "create temporary file: {error}"
                )))
            }
        }
    }
    Err(WorkspaceFsError::Io(
        "could not allocate a unique sibling temporary file".to_string(),
    ))
}

struct TemporaryGuard(PathBuf);

impl TemporaryGuard {
    fn disarm(mut self) {
        self.0.clear();
    }
}

impl Drop for TemporaryGuard {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.0);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn publish_file(
    source: &Path,
    destination: &Path,
    mode: AtomicPublishMode,
) -> Result<(), WorkspaceFsError> {
    match mode {
        AtomicPublishMode::ReplaceExisting => fs::rename(source, destination)
            .map_err(|error| WorkspaceFsError::Io(format!("atomic replace: {error}"))),
        AtomicPublishMode::CreateNew => {
            use rustix::fs::{renameat_with, RenameFlags, CWD};
            renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE)
                .map_err(|error| WorkspaceFsError::Io(format!("atomic create: {error}")))
        }
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
fn publish_file(
    source: &Path,
    destination: &Path,
    mode: AtomicPublishMode,
) -> Result<(), WorkspaceFsError> {
    match mode {
        AtomicPublishMode::ReplaceExisting => fs::rename(source, destination)
            .map_err(|error| WorkspaceFsError::Io(format!("atomic replace: {error}"))),
        AtomicPublishMode::CreateNew => {
            fs::hard_link(source, destination)
                .map_err(|error| WorkspaceFsError::Io(format!("atomic create: {error}")))?;
            fs::remove_file(source).map_err(|error| {
                WorkspaceFsError::Io(format!("remove published staging link: {error}"))
            })
        }
    }
}

#[cfg(windows)]
fn publish_file(
    source: &Path,
    destination: &Path,
    mode: AtomicPublishMode,
) -> Result<(), WorkspaceFsError> {
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
    let flags = MOVEFILE_WRITE_THROUGH
        | if matches!(mode, AtomicPublishMode::ReplaceExisting) {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    let result = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) };
    if result == 0 {
        Err(WorkspaceFsError::Io(format!(
            "atomic replace: {}",
            std::io::Error::last_os_error()
        )))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<(), WorkspaceFsError> {
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|error| WorkspaceFsError::Io(format!("sync parent directory: {error}")))
}

#[cfg(windows)]
fn sync_parent(_parent: &Path) -> Result<(), WorkspaceFsError> {
    Ok(())
}
