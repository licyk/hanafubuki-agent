//! Shared use of a data directory. Individual stores coordinate their writes
//! and maintenance; sessions and process records retain independent owners.
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
    /// Shares `directory` with other Agents. An exclusive maintenance lock
    /// (including an older Agent's directory lock) still refuses the open.
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
        match file.try_lock_shared() {
            Ok(()) => Ok(Self {
                file,
                directory: directory.to_path_buf(),
            }),
            Err(TryLockError::WouldBlock) => Err(format!(
                "the Agent data directory {} is exclusively locked by another process ({}); \
                 close that older Agent or wait for its maintenance to finish",
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
