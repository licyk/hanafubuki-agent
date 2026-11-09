//! One Agent process per data directory.
//!
//! The history's SQLite index and its per-session writers, the memory store
//! and the process-group ledger all assume a single writer process, and a
//! start reaps the process groups the ledger lists: two processes on one
//! data directory would corrupt the history and terminate each other's
//! processes. The lock is advisory (`flock` on Unix, `LockFileEx` on
//! Windows) and the operating system drops it when the process ends, so a
//! crash leaves nothing to clean up.
//!
//! Dropping the lock unlocks the file before closing it: closing alone does
//! not release the lock while another descriptor of the same open file is
//! alive, and a process forked to spawn a child (by the Agent, its host or a
//! test beside it) holds a copy of every descriptor until it execs.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

/// The lock file in the data directory.
pub const DATA_DIR_LOCK_FILE: &str = ".lock";

/// The data directory's lock, held until it is dropped.
#[derive(Debug)]
pub struct DataDirLock {
    file: File,
    directory: PathBuf,
}

impl DataDirLock {
    /// Takes the lock of `directory`, creating both if needed; fails at
    /// once when another process holds it.
    pub fn acquire(directory: &Path) -> Result<Self, String> {
        let path = directory.join(DATA_DIR_LOCK_FILE);
        fs::create_dir_all(directory)
            .map_err(|error| format!("create {}: {error}", directory.display()))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| format!("open {}: {error}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Self {
                file,
                directory: directory.to_path_buf(),
            }),
            Err(TryLockError::WouldBlock) => Err(format!(
                "the Agent data directory {} is in use by another process (it holds {}); \
                 only one Agent may use a data directory at a time",
                directory.display(),
                path.display()
            )),
            Err(TryLockError::Error(error)) => Err(format!("lock {}: {error}", path.display())),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for DataDirLock {
    fn drop(&mut self) {
        if let Err(error) = self.file.unlock() {
            log::warn!(
                "the Agent data directory {} was not unlocked: {error}",
                self.directory.display()
            );
        }
    }
}

#[cfg(test)]
mod tests;
