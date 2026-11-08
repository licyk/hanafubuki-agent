use crate::model::{WorkspaceFsError, WorkspacePathKind};
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) fn move_no_replace(
    source: &Path,
    destination: &Path,
    _kind: WorkspacePathKind,
) -> Result<(), WorkspaceFsError> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};

    renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE)
        .map_err(|error| WorkspaceFsError::Io(format!("move path without replacement: {error}")))
}

#[cfg(windows)]
pub(super) fn move_no_replace(
    source: &Path,
    destination: &Path,
    _kind: WorkspacePathKind,
) -> Result<(), WorkspaceFsError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

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
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(WorkspaceFsError::Io(format!(
            "move path without replacement: {}",
            std::io::Error::last_os_error()
        )))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub(super) fn move_no_replace(
    source: &Path,
    destination: &Path,
    _kind: WorkspacePathKind,
) -> Result<(), WorkspaceFsError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    const RENAME_EXCL: u32 = 0x0000_0004;
    extern "C" {
        fn renamex_np(
            old: *const std::ffi::c_char,
            new: *const std::ffi::c_char,
            flags: u32,
        ) -> std::ffi::c_int;
    }
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| WorkspaceFsError::InvalidRequest("source path contains NUL".to_string()))?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        WorkspaceFsError::InvalidRequest("destination path contains NUL".to_string())
    })?;
    if unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(WorkspaceFsError::Io(format!(
            "move path without replacement: {}",
            std::io::Error::last_os_error()
        )))
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
pub(super) fn move_no_replace(
    source: &Path,
    destination: &Path,
    kind: WorkspacePathKind,
) -> Result<(), WorkspaceFsError> {
    if kind == WorkspacePathKind::Directory {
        return Err(WorkspaceFsError::Io(
            "atomic no-replace directory move is unsupported on this platform".to_string(),
        ));
    }
    fs::hard_link(source, destination)
        .map_err(|error| WorkspaceFsError::Io(format!("move path link: {error}")))?;
    fs::remove_file(source)
        .map_err(|error| WorkspaceFsError::Io(format!("move path unlink: {error}")))
}
