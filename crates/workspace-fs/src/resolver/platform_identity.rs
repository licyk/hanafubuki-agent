use crate::model::WorkspaceFsError;
use std::fs;
use std::path::Path;

#[cfg(unix)]
pub(super) fn platform_identity(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<(u64, u64), WorkspaceFsError> {
    use std::os::unix::fs::MetadataExt;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
pub(super) fn platform_identity(
    path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(u64, u64), WorkspaceFsError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };

    let path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(WorkspaceFsError::Resolution(format!(
            "open path identity: {}",
            std::io::Error::last_os_error()
        )));
    }
    let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let result = unsafe { GetFileInformationByHandle(handle, &mut information) };
    unsafe {
        CloseHandle(handle);
    }
    if result == 0 {
        return Err(WorkspaceFsError::Resolution(format!(
            "read path identity: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok((
        information.dwVolumeSerialNumber as u64,
        ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
    ))
}
