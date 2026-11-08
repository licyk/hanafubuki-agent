use std::path::Path;

use hanafubuki_agent_kernel::AgentHistoryStoreError;

#[cfg(not(windows))]
pub(super) fn publish_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
pub(super) fn publish_file(source: &Path, destination: &Path) -> std::io::Result<()> {
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
pub(super) fn sync_directory(path: &Path) -> Result<(), AgentHistoryStoreError> {
    std::fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(super::io_error)
}

// std::fs::File::open cannot open a directory on Windows without native
// FILE_FLAG_BACKUP_SEMANTICS. Windows publication uses MOVEFILE_WRITE_THROUGH;
// directory fsync remains an additional Unix durability boundary.
#[cfg(not(unix))]
pub(super) fn sync_directory(_path: &Path) -> Result<(), AgentHistoryStoreError> {
    Ok(())
}

#[cfg(test)]
mod tests;
