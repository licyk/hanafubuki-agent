//! Advisory file locks shared by processes. Lock files are never unlinked:
//! replacing an inode would let two processes lock different files at one path.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

use crate::paths::{system_form, ResolvedPath};

#[derive(Debug)]
pub struct FileLock(File);

impl FileLock {
    pub fn exclusive(path: &Path) -> io::Result<Self> {
        let file = open(path)?;
        file.lock()?;
        Ok(Self(file))
    }

    pub fn shared(path: &Path) -> io::Result<Self> {
        let file = open(path)?;
        file.lock_shared()?;
        Ok(Self(file))
    }

    pub fn try_exclusive(path: &Path) -> io::Result<Option<Self>> {
        let file = open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self(file))),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

fn open(path: &Path) -> io::Result<File> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(system_form(parent).as_ref())?;
    let parent = ResolvedPath::resolve(parent).map_err(io::Error::other)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("a lock must name a file"))?;
    let path = ResolvedPath::resolve_child(&parent, name).map_err(io::Error::other)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.system())
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Explicit unlock also releases a descriptor inherited by a child
        // between fork and exec, as it does for the Agent's lifetime locks.
        if let Err(error) = self.0.unlock() {
            log::warn!("file lock was not released: {error}");
        }
    }
}
