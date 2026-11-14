//! Putting the staged executable in place (D5): never a write into the
//! installed file and never a process closed to free it. On Unix one
//! `rename` over the real path: every running process keeps its inode, and
//! there is no moment without an executable. On Windows, where a running
//! executable can be renamed but not replaced, it is moved aside to
//! `<name>.<unix ms>.outdated` and the new one renamed in, the old one put
//! back when that fails.

use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;

use crate::error::UpdateError;

#[cfg(unix)]
mod unix;
#[cfg(any(windows, test))]
pub(crate) mod windows;

/// Puts `staged` (in the executable's directory) at `executable`; on
/// Windows, returns where the previous executable was moved.
pub(crate) fn replace(
    executable: &ResolvedPath,
    staged: &Path,
) -> Result<Option<PathBuf>, UpdateError> {
    #[cfg(unix)]
    {
        unix::replace(executable, staged).map(|()| None)
    }
    #[cfg(windows)]
    {
        let backup = windows::Backup::of(executable);
        windows::replace(executable, staged, &backup, &windows::rename_retrying)?;
        Ok(Some(backup.native))
    }
}
