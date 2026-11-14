//! What earlier updates left beside the executable (D18): a staging
//! directory an interrupted update did not remove, and on Windows the
//! previous executables moved aside, once no process runs them.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use crate::install::Installation;

/// The prefix of an update's staging directory beside the executable.
/// `Installation::writable`'s probe shares it, but is a file.
pub(crate) const STAGING_PREFIX: &str = ".hanafubuki-agent-update-";
/// The suffix of an executable moved aside on Windows:
/// `<name>.<unix milliseconds>.outdated`.
pub(crate) const OUTDATED_SUFFIX: &str = ".outdated";

/// Removes, best effort, the previous executables moved aside beside this
/// one that no process runs any more. Windows only: Unix never moves one
/// aside. One directory read.
pub fn sweep_outdated(installation: &Installation) {
    if cfg!(windows) {
        let executable = installation.executable();
        if let (Some(directory), Some(name)) =
            (executable.system().parent(), executable.file_name())
        {
            remove_outdated(directory, name);
        }
    }
}

/// Removes every staging directory in `directory`. Called under the update
/// lock, so none belongs to a live update: one is created only under it.
pub(crate) fn remove_stale_staging(directory: &Path) {
    for entry in entries(directory) {
        let is_staging = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(STAGING_PREFIX));
        let is_directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_staging && is_directory {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Removes the backups of `executable` (a file name) in `directory`; one
/// still mapped by a running process cannot be removed and stays.
pub(crate) fn remove_outdated(directory: &Path, executable: &OsStr) {
    let Some(executable) = executable.to_str() else {
        return;
    };
    for entry in entries(directory) {
        let is_backup = entry
            .file_name()
            .to_str()
            .is_some_and(|name| is_backup_of(name, executable));
        let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
        if is_backup && is_file {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Whether `name` is `<executable>.<digits>.outdated`.
pub(crate) fn is_backup_of(name: &str, executable: &str) -> bool {
    name.strip_prefix(executable)
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_suffix(OUTDATED_SUFFIX))
        .is_some_and(|stamp| !stamp.is_empty() && stamp.bytes().all(|byte| byte.is_ascii_digit()))
}

fn entries(directory: &Path) -> impl Iterator<Item = fs::DirEntry> {
    fs::read_dir(directory).into_iter().flatten().flatten()
}

#[cfg(test)]
mod tests;
