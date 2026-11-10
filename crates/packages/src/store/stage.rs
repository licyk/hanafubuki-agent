// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/store.rs` (`replace_plugin_root_atomically`:
// the copy into staging, the fallback manifest written into the copy, the
// staged manifest checked against the source, the commands migrated, the
// swap that keeps the previous version until it succeeds, the pruning of
// older versions) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: staging is the store's own area; `.git`
// directories are not copied; a failed command migration is an issue of
// the install.

use std::fs;
use std::path::{Path, PathBuf};

use super::{io_error, PackageStore};
use crate::commands::migrate_commands;
use crate::manifest::{find_manifest, ManifestFormat};
use crate::version::{compare_versions, validate_version_segment, LOCAL_VERSION};
use crate::PackageError;

/// The manifest an install stages.
pub(super) enum StagedManifest<'a> {
    /// The package's own.
    OnDisk(PathBuf),
    /// A marketplace entry's, written as `.codex-plugin/plugin.json`.
    Fallback(&'a str),
}

pub(super) struct Staged {
    pub(super) format: ManifestFormat,
    pub(super) migrated_commands: Vec<String>,
    pub(super) issues: Vec<String>,
}

/// Puts a copy of `source` at `base/version`. A new version is renamed in
/// beside the old ones, which are then pruned; the same version replaces
/// the whole `base`, the previous one kept until the swap succeeded.
pub(super) fn replace_root(
    store: &PackageStore,
    source: &Path,
    base: &Path,
    version: &str,
    manifest: &StagedManifest<'_>,
    commands: &[PathBuf],
) -> Result<Staged, PackageError> {
    let staging = store.staging("install-")?;
    let staged_base = staging.path().join("package");
    let staged = staged_base.join(version);
    copy_tree(source, &staged)?;
    let (relative, expected) = match manifest {
        StagedManifest::OnDisk(path) => {
            let relative = path.strip_prefix(source).map_err(|_| {
                PackageError::Invalid("plugin manifest is outside the plugin source".to_string())
            })?;
            let bytes = fs::read(path).map_err(|error| io_error("read plugin.json", error))?;
            (relative.to_path_buf(), bytes)
        }
        StagedManifest::Fallback(contents) => {
            let relative = PathBuf::from(ManifestFormat::Codex.relative_path());
            write_fallback(&staged.join(&relative), contents)?;
            (relative, contents.as_bytes().to_vec())
        }
    };
    let format = verify_staged_manifest(&staged, &relative, &expected)?;
    let mut issues = Vec::new();
    let migrated_commands = if format == ManifestFormat::AgentPlugins {
        Vec::new()
    } else {
        let commands = commands
            .iter()
            .filter_map(|path| path.strip_prefix(source).ok())
            .map(|relative| staged.join(relative))
            .collect::<Vec<_>>();
        migrate_commands(&staged, &commands).unwrap_or_else(|error| {
            issues.push(format!(
                "the commands were not migrated into Skills: {error}"
            ));
            Vec::new()
        })
    };
    activate(&staging, &staged_base, base, version)?;
    Ok(Staged {
        format,
        migrated_commands,
        issues,
    })
}

fn write_fallback(path: &Path, contents: &str) -> Result<(), PackageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create the plugin manifest directory", error))?;
    }
    fs::write(path, contents).map_err(|error| io_error("write the fallback plugin manifest", error))
}

/// The staged copy's manifest must be the one the install read, byte for
/// byte, where the install read it.
fn verify_staged_manifest(
    staged: &Path,
    relative: &Path,
    expected: &[u8],
) -> Result<ManifestFormat, PackageError> {
    let (format, path) = find_manifest(staged).ok_or_else(|| {
        PackageError::Invalid("plugin manifest is missing after installation staging".to_string())
    })?;
    if path != staged.join(relative) {
        return Err(PackageError::Invalid(
            "plugin manifest changed during installation staging".to_string(),
        ));
    }
    let staged_bytes =
        fs::read(&path).map_err(|error| io_error("read staged plugin.json", error))?;
    if staged_bytes != expected {
        return Err(PackageError::Invalid(
            "plugin manifest contents changed during installation staging".to_string(),
        ));
    }
    Ok(format)
}

fn activate(
    staging: &tempfile::TempDir,
    staged_base: &Path,
    base: &Path,
    version: &str,
) -> Result<(), PackageError> {
    if let Some(parent) = base.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create the plugin cache directory", error))?;
    }
    let target_version = base.join(version);
    if base.exists() && !target_version.exists() {
        fs::rename(staged_base.join(version), &target_version)
            .map_err(|error| io_error("activate the new plugin version", error))?;
        return prune_old_versions(base, version);
    }
    if !base.exists() {
        return fs::rename(staged_base, base)
            .map_err(|error| io_error("activate the plugin cache entry", error));
    }
    let backup = staging.path().join("previous");
    fs::rename(base, &backup).map_err(|error| io_error("back up the plugin cache entry", error))?;
    if let Err(error) = fs::rename(staged_base, base) {
        return match fs::rename(&backup, base) {
            Ok(()) => Err(io_error("activate the updated plugin cache entry", error)),
            Err(rollback) => {
                let kept = keep_backup(&backup, base);
                Err(PackageError::Io(format!(
                    "failed to activate the updated plugin cache entry at {}: {error}; failed to restore the previous one (left at {}): {rollback}",
                    base.display(),
                    kept.display()
                )))
            }
        };
    }
    Ok(())
}

/// Moves a backup that could not be restored out of the staging area, which
/// is cleared, beside the entry it backs up.
fn keep_backup(backup: &Path, base: &Path) -> PathBuf {
    let mut kept = base.as_os_str().to_owned();
    kept.push(format!(".previous-{}", std::process::id()));
    let kept = PathBuf::from(kept);
    match fs::rename(backup, &kept) {
        Ok(()) => kept,
        Err(_) => backup.to_path_buf(),
    }
}

fn prune_old_versions(base: &Path, version: &str) -> Result<(), PackageError> {
    let Ok(entries) = fs::read_dir(base) else {
        return Ok(());
    };
    for entry in entries.filter_map(Result::ok) {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Ok(old) = entry.file_name().into_string() else {
            continue;
        };
        if old == version || validate_version_segment(&old).is_err() {
            continue;
        }
        if fs::remove_dir_all(entry.path()).is_err() && would_stay_active(&old, version) {
            return Err(PackageError::Io(format!(
                "failed to activate plugin version `{version}` while `{old}` remains active"
            )));
        }
    }
    Ok(())
}

fn would_stay_active(old: &str, new: &str) -> bool {
    old == LOCAL_VERSION || compare_versions(old, new).is_gt()
}

/// Copies directories and regular files; symbolic links and `.git` are
/// left out.
fn copy_tree(source: &Path, target: &Path) -> Result<(), PackageError> {
    fs::create_dir_all(target)
        .map_err(|error| io_error("create the plugin target directory", error))?;
    let entries = fs::read_dir(source)
        .map_err(|error| io_error("read the plugin source directory", error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error("list the plugin source", error))?;
        let kind = entry
            .file_type()
            .map_err(|error| io_error("inspect a plugin source entry", error))?;
        let target = target.join(entry.file_name());
        if kind.is_dir() && entry.file_name() != ".git" {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)
                .map_err(|error| io_error("copy a plugin file", error))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
