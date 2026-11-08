//! Small filesystem primitives for destructively replacing incompatible state.
//!
//! Schema policy stays in the owning domain. This module only moves an invalid
//! file out of the live location so the owner can publish one canonical current
//! document without leaving an incompatible backup in the normal read path.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_QUARANTINE: AtomicU64 = AtomicU64::new(1);

/// Move `path` into a sibling `.recovery` directory. Missing files are ordinary.
pub fn quarantine_file(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    let recovery = parent.join(".recovery");
    fs::create_dir_all(&recovery)
        .map_err(|error| format!("create {}: {error}", recovery.display()))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("state");
    let target = recovery.join(format!(
        "{name}.{}.{}.invalid",
        std::process::id(),
        NEXT_QUARANTINE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::rename(path, &target).map_err(|error| {
        format!(
            "quarantine {} -> {}: {error}",
            path.display(),
            target.display()
        )
    })?;
    Ok(Some(target))
}

/// Quarantine the live JSON document and the backup used by `safe_file`.
pub fn quarantine_safe_json(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut quarantined = Vec::new();
    if let Some(path) = quarantine_file(path)? {
        quarantined.push(path);
    }
    let mut backup = path.as_os_str().to_owned();
    backup.push(".bak");
    if let Some(path) = quarantine_file(&PathBuf::from(backup))? {
        quarantined.push(path);
    }
    Ok(quarantined)
}

/// Quarantine a SQLite database and its WAL/shared-memory sidecars.
pub fn quarantine_sqlite(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut quarantined = Vec::new();
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let mut shm = path.as_os_str().to_owned();
    shm.push("-shm");
    for candidate in [path.to_path_buf(), PathBuf::from(wal), PathBuf::from(shm)] {
        if let Some(path) = quarantine_file(&candidate)? {
            quarantined.push(path);
        }
    }
    Ok(quarantined)
}

/// Permanently remove a rebuildable SQLite database and its sidecars.
pub fn discard_sqlite(path: &Path) -> Result<(), String> {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let mut shm = path.as_os_str().to_owned();
    shm.push("-shm");
    for candidate in [path.to_path_buf(), PathBuf::from(wal), PathBuf::from(shm)] {
        match fs::remove_file(&candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("remove {}: {error}", candidate.display())),
        }
    }
    Ok(())
}
