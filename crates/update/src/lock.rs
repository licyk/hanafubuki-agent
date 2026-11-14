//! One update of an executable at a time (D6): a lock file beside it,
//! `.<name>.update.lock`, taken without waiting. The file stays after the
//! update: removing it would race a second updater that just opened it.

use std::fs::{File, TryLockError};

use hanafubuki_agent_foundation::paths::ResolvedPath;

use crate::error::UpdateError;

/// The held lock; dropping it releases it.
#[derive(Debug)]
pub(crate) struct UpdateLock {
    file: File,
}

impl UpdateLock {
    /// Takes the lock of `executable`, or fails at once when another
    /// update holds it.
    pub(crate) fn acquire(executable: &ResolvedPath) -> Result<Self, UpdateError> {
        let name = lock_name(executable);
        let directory = executable.native().parent().unwrap_or(executable.native());
        let failed = |reason: String| UpdateError::Write {
            what: "the update lock",
            directory: directory.display().to_string(),
            reason,
        };
        let path = executable.system().with_file_name(&name);
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|error| failed(error.to_string()))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { file }),
            Err(TryLockError::WouldBlock) => Err(UpdateError::LockHeld {
                executable: executable.native_text().to_string(),
                lock: name,
            }),
            Err(TryLockError::Error(error)) => Err(failed(error.to_string())),
        }
    }
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        // A child process spawned meanwhile may hold a duplicate of the
        // descriptor until it execs; unlocking ends the update's hold
        // whatever the duplicates.
        let _ = self.file.unlock();
    }
}

/// `.<file name>.update.lock`.
pub(crate) fn lock_name(executable: &ResolvedPath) -> String {
    let file_name = executable
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!(".{file_name}.update.lock")
}

#[cfg(test)]
mod tests;
