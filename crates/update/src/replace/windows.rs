//! Windows: the running executable is moved aside, the new one renamed
//! in, and the old one put back if that fails. Each rename is retried for
//! two seconds on a sharing violation, which an antivirus scan of the file
//! just written commonly causes (§6 Q7). Compiled everywhere for the tests:
//! its renames are plain `rename`s.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hanafubuki_agent_foundation::paths::ResolvedPath;

use crate::cleanup::OUTDATED_SUFFIX;
use crate::error::UpdateError;

/// `ERROR_ACCESS_DENIED` and `ERROR_SHARING_VIOLATION`: another program
/// holds the file.
const HELD_ERRORS: [i32; 2] = [5, 32];
/// Ten attempts, two seconds in all.
const ATTEMPTS: u32 = 10;
const PAUSE: Duration = Duration::from_millis(200);

/// Where the running executable is moved: `<name>.<unix ms>.outdated`
/// beside it.
pub(crate) struct Backup {
    pub(crate) system: PathBuf,
    pub(crate) native: PathBuf,
}

impl Backup {
    pub(crate) fn of(executable: &ResolvedPath) -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or_default();
        let name = executable
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = format!("{name}.{millis}{OUTDATED_SUFFIX}");
        Self {
            system: executable.system().with_file_name(&name),
            native: executable.native().with_file_name(&name),
        }
    }
}

/// Moves `executable` to `backup`, then `staged` to `executable`, with
/// `rename`; when the second fails, moves the backup back.
pub(crate) fn replace(
    executable: &ResolvedPath,
    staged: &Path,
    backup: &Backup,
    rename: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), UpdateError> {
    let path = executable.system();
    let shown = || executable.native_text().to_string();
    rename(path, &backup.system).map_err(|error| UpdateError::MoveAside {
        executable: shown(),
        held: is_held(&error),
        reason: error.to_string(),
    })?;
    let Err(error) = rename(staged, path) else {
        return Ok(());
    };
    match rename(&backup.system, path) {
        Ok(()) => Err(UpdateError::Restored {
            executable: shown(),
            reason: error.to_string(),
        }),
        Err(rollback) => Err(UpdateError::NotRestored {
            executable: shown(),
            backup: backup.native.display().to_string(),
            reason: error.to_string(),
            rollback: rollback.to_string(),
        }),
    }
}

/// `std::fs::rename`, retried while another program holds the file.
pub(crate) fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    retry(ATTEMPTS, PAUSE, || std::fs::rename(from, to))
}

/// `operation`, tried up to `attempts` times, `pause` apart, while it fails
/// because another program holds the file.
pub(crate) fn retry(
    attempts: u32,
    pause: Duration,
    mut operation: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let mut attempt = 1;
    loop {
        match operation() {
            Err(error) if is_held(&error) && attempt < attempts => {
                attempt += 1;
                std::thread::sleep(pause);
            }
            result => return result,
        }
    }
}

fn is_held(error: &io::Error) -> bool {
    error
        .raw_os_error()
        .is_some_and(|code| HELD_ERRORS.contains(&code))
}

#[cfg(test)]
mod tests;
