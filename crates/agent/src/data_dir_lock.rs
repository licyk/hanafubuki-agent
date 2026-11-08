//! One Agent process per data directory.
//!
//! The history's SQLite index and its per-session writers, the memory store
//! and the process-group ledger all assume a single writer process, and a
//! start reaps the process groups the ledger lists: two processes on one
//! data directory would corrupt the history and terminate each other's
//! processes. The lock is advisory (`flock` on Unix, `LockFileEx` on
//! Windows) and the operating system drops it when the process ends, so a
//! crash leaves nothing to clean up.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

/// The lock file in the data directory.
pub const DATA_DIR_LOCK_FILE: &str = ".lock";

/// The data directory's lock, held until it is dropped.
#[derive(Debug)]
pub struct DataDirLock {
    _file: File,
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
                _file: file,
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

#[cfg(test)]
mod tests;
