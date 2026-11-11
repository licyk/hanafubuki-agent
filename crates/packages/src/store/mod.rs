// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/store.rs` (the versioned cache
// `cache/<marketplace>/<plugin>/<version>`, the data directories as Codex
// names them, the active installation, uninstall) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the store's root is the data directory's
// `plugins/`, which also holds the git-added marketplaces, the staging area
// and the install records; installed packages are listed with their
// manifest and record, and so are the directories linked for development,
// also without opening a store; uninstall can purge the data directory;
// the remote install metadata is not ported.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use hanafubuki_agent_process::executor::ExecutorManager;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::id::{validate_marketplace_name, validate_plugin_name};
use crate::manifest::{load_manifest, LoadedManifest, ManifestFormat, PackagePermissions};
use crate::version::{active_version, LOCAL_VERSION};
use crate::{PackageError, PluginId};

mod install;
mod link;
mod record;
mod stage;

pub use install::InstallOutcome;
pub use link::LinkOutcome;
pub use record::InstallRecord;
use record::{LinkedRecord, Records};

const CACHE_DIR: &str = "cache";
const DATA_DIR: &str = "data";
const AGENT_PLUGINS_DATA_DIR: &str = "agent-plugins";
const MARKETPLACES_DIR: &str = "marketplaces";
const STAGING_DIR: &str = ".staging";

/// The installed packages under the data directory's `plugins/`:
///
/// ```text
/// cache/<marketplace>/<plugin>/<version>/   a package as installed
/// data/<plugin>-<marketplace>/              its own data (PLUGIN_DATA)
/// data/agent-plugins/<sha256>/              the same for Agent Plugins
/// marketplaces/<name>/                      a git-added marketplace
/// install.json                              the cache's packages' records
/// linked.json                               the linked packages' records
/// .staging/                                 work in progress
/// ```
///
/// A package linked for development (`plugins link`) has no directory in
/// the cache: its `linked.json` record names the directory that is the
/// package.
///
/// One store per data directory: opening one sweeps the staging area, and
/// its changes are serialized among themselves. A reader beside a running
/// Agent lists the packages with [`PackageStore::read_installed`] instead.
pub struct PackageStore {
    root: PathBuf,
    executor: ExecutorManager,
    changes: Mutex<()>,
}

/// A package in the cache, or linked.
#[derive(Clone, Debug, PartialEq)]
pub struct InstalledPackage {
    pub id: PluginId,
    /// The version in use (`local`, else the highest); a linked package's
    /// manifest's, else `local`.
    pub version: String,
    /// Its directory in the cache, or the linked directory.
    pub root: PathBuf,
    /// Its manifest, when it still reads.
    pub manifest: Option<LoadedManifest>,
    pub record: Option<InstallRecord>,
    pub data_root: PathBuf,
    /// What is wrong with it: a manifest that does not read, a missing
    /// record.
    pub issues: Vec<String>,
}

impl InstalledPackage {
    /// The directory it is linked to, when it is linked.
    pub fn linked(&self) -> Option<&Path> {
        self.record.as_ref().and_then(InstallRecord::linked)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UninstallOutcome {
    pub id: PluginId,
    /// Whether the package was installed: in the cache, or linked (its
    /// directory is never removed).
    pub removed: bool,
    /// The data directory deleted by a purge.
    pub purged_data: Option<PathBuf>,
}

impl PackageStore {
    /// The store at `root` (the data directory's `plugins/`), running `git`
    /// through `executor`.
    pub fn open(root: PathBuf, executor: ExecutorManager) -> Self {
        let staging = root.join(STAGING_DIR);
        if let Err(error) = fs::remove_dir_all(&staging) {
            if error.kind() != ErrorKind::NotFound {
                log::warn!(
                    "the plugin staging area {} was not cleared: {error}",
                    staging.display()
                );
            }
        }
        Self {
            root,
            executor,
            changes: Mutex::new(()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn package_base(&self, id: &PluginId) -> PathBuf {
        package_base(&self.root, id)
    }

    pub fn package_root(&self, id: &PluginId, version: &str) -> PathBuf {
        self.package_base(id).join(version)
    }

    pub fn active_version(&self, id: &PluginId) -> Option<String> {
        active_version(&self.package_base(id))
    }

    pub fn active_root(&self, id: &PluginId) -> Option<PathBuf> {
        self.active_version(id)
            .map(|version| self.package_root(id, &version))
    }

    /// The package's own data directory, which uninstall keeps: Codex's
    /// `data/<plugin>-<marketplace>`, or for an Agent Plugins package
    /// `data/agent-plugins/<sha256 of marketplace NUL plugin>`.
    pub fn data_root(&self, id: &PluginId, format: ManifestFormat) -> PathBuf {
        data_root(&self.root, id, format)
    }

    /// Where a git-added marketplace is checked out.
    pub fn marketplace_checkout(&self, name: &str) -> PathBuf {
        self.root.join(MARKETPLACES_DIR).join(name)
    }

    /// Every package in the cache, by id, then every linked one.
    pub fn installed(&self) -> Vec<InstalledPackage> {
        list(&self.root, &self.records())
    }

    /// The packages installed under `root` (the data directory's
    /// `plugins/`), as [`Self::installed`] lists them, read without
    /// opening a store: the staging area is not swept, a record that does
    /// not read is not quarantined, and nothing is written or locked, so a
    /// command may read them beside a running Agent.
    pub fn read_installed(root: &Path) -> Vec<InstalledPackage> {
        list(root, &Records::read(root))
    }

    /// The installed package `id`.
    pub fn read(&self, id: &PluginId) -> Result<InstalledPackage, PackageError> {
        let records = self.records();
        cached(&self.root, id.clone(), &records)
            .or_else(|| linked(&self.root, id.clone(), &records))
            .ok_or_else(|| PackageError::NotFound(format!("plugin `{id}` is not installed")))
    }

    /// The directory the package `id` is linked to, when it is linked.
    pub(crate) fn linked_path(&self, id: &PluginId) -> Option<PathBuf> {
        self.records()
            .linked
            .get(id)
            .map(|record| record.path.clone())
    }

    /// Removes the package `id` from the cache and the records. Its data
    /// directory is kept unless `purge_data`. A linked package's record
    /// goes, and its directory stays as it is.
    pub fn uninstall(
        &self,
        id: &PluginId,
        purge_data: bool,
    ) -> Result<UninstallOutcome, PackageError> {
        let _change = self.changes.lock();
        let records = self.records();
        let linked = records.linked.get(id).map(|record| record.path.clone());
        let format = self
            .active_root(id)
            .or(linked.clone())
            .and_then(|root| load_manifest(&root).ok())
            .map(|manifest| manifest.format)
            .or(records.installed.get(id).and_then(|record| record.format))
            .or(records.linked.get(id).and_then(|record| record.format))
            .unwrap_or(ManifestFormat::Codex);
        let base = self.package_base(id);
        let removed = remove_existing(&base)? || linked.is_some();
        self.update_records(|records| {
            records.installed.remove(id);
            records.linked.remove(id);
        })?;
        let data = self.data_root(id, format);
        let purged_data = if purge_data && remove_existing(&data)? {
            Some(data)
        } else {
            None
        };
        Ok(UninstallOutcome {
            id: id.clone(),
            removed,
            purged_data,
        })
    }

    /// Records that the user granted the code of the installed package `id`
    /// `permissions`, replacing an earlier grant.
    pub fn grant_permissions(
        &self,
        id: &PluginId,
        permissions: PackagePermissions,
    ) -> Result<(), PackageError> {
        let _change = self.changes.lock();
        let cached = self.active_version(id).is_some();
        let mut found = false;
        self.update_records(|records| {
            let granted = match records.linked.get_mut(id) {
                Some(linked) if !cached => &mut linked.granted_permissions,
                _ => match records.installed.get_mut(id) {
                    Some(record) => &mut record.granted_permissions,
                    None => return,
                },
            };
            *granted = Some(permissions);
            found = true;
        })?;
        if !found {
            return Err(PackageError::NotFound(format!(
                "plugin `{id}` has no install record"
            )));
        }
        log::info!("plugin {id} permissions granted");
        Ok(())
    }

    pub(crate) fn executor(&self) -> &ExecutorManager {
        &self.executor
    }

    pub(crate) fn changes(&self) -> parking_lot::MutexGuard<'_, ()> {
        self.changes.lock()
    }

    /// A fresh directory in the staging area, removed when dropped.
    pub(crate) fn staging(&self, prefix: &str) -> Result<tempfile::TempDir, PackageError> {
        let staging = self.root.join(STAGING_DIR);
        fs::create_dir_all(&staging).map_err(|error| io_error("create the staging area", error))?;
        tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(&staging)
            .map_err(|error| io_error("create a staging directory", error))
    }

    /// Both record files, a file that does not read quarantined.
    fn records(&self) -> Records {
        let records = Records::read(&self.root);
        records.quarantine_unreadable();
        records
    }

    pub(crate) fn update_records(
        &self,
        change: impl FnOnce(&mut Records),
    ) -> Result<(), PackageError> {
        let mut records = self.records();
        change(&mut records);
        records.write(&self.root).map_err(PackageError::Io)
    }
}

/// Every package in the cache under `root`, by id, then every linked one
/// whose id the cache does not hold.
fn list(root: &Path, records: &Records) -> Vec<InstalledPackage> {
    let mut packages = Vec::new();
    for marketplace in directory_names(&root.join(CACHE_DIR)) {
        if validate_marketplace_name(&marketplace).is_err() {
            continue;
        }
        let base = root.join(CACHE_DIR).join(&marketplace);
        for plugin in directory_names(&base) {
            let Ok(id) = PluginId::new(&plugin, &marketplace) else {
                continue;
            };
            packages.extend(cached(root, id, records));
        }
    }
    let cached = packages.len();
    for id in records.linked.keys() {
        if !packages[..cached].iter().any(|package| package.id == *id) {
            packages.extend(linked(root, id.clone(), records));
        }
    }
    packages
}

/// The package `id` in the cache under `root`.
fn cached(root: &Path, id: PluginId, records: &Records) -> Option<InstalledPackage> {
    let base = package_base(root, &id);
    let version = active_version(&base)?;
    let root_of_version = base.join(&version);
    let mut issues = Vec::new();
    let manifest = match load_manifest(&root_of_version) {
        Ok(manifest) => Some(manifest),
        Err(error) => {
            issues.push(error.to_string());
            None
        }
    };
    let record = records.installed.get(&id).cloned();
    if record.is_none() {
        issues.push("the install record has no entry for it".to_string());
    }
    let format = manifest
        .as_ref()
        .map(|manifest| manifest.format)
        .or(record.as_ref().and_then(|record| record.format))
        .unwrap_or(ManifestFormat::Codex);
    Some(InstalledPackage {
        data_root: data_root(root, &id, format),
        id,
        version,
        root: root_of_version,
        manifest,
        record,
        issues,
    })
}

/// The package `id` linked to the directory its record names, read as that
/// directory is now; one whose directory does not read is listed with the
/// issue.
fn linked(root: &Path, id: PluginId, records: &Records) -> Option<InstalledPackage> {
    let record: &LinkedRecord = records.linked.get(&id)?;
    let directory = record.path.clone();
    let mut issues = Vec::new();
    let manifest = match load_manifest(&directory) {
        Ok(manifest) => Some(manifest),
        Err(error) => {
            issues.push(format!(
                "its linked directory {} does not read: {error}",
                directory.display()
            ));
            None
        }
    };
    let version = manifest
        .as_ref()
        .and_then(|manifest| manifest.manifest.version.clone())
        .unwrap_or_else(|| LOCAL_VERSION.to_string());
    let format = manifest
        .as_ref()
        .map(|manifest| manifest.format)
        .or(record.format)
        .unwrap_or(ManifestFormat::Codex);
    Some(InstalledPackage {
        data_root: data_root(root, &id, format),
        id,
        version,
        root: directory,
        manifest,
        record: Some(record.install_record()),
        issues,
    })
}

fn package_base(root: &Path, id: &PluginId) -> PathBuf {
    root.join(CACHE_DIR)
        .join(&id.marketplace_name)
        .join(&id.plugin_name)
}

fn data_root(root: &Path, id: &PluginId, format: ManifestFormat) -> PathBuf {
    let data = root.join(DATA_DIR);
    match format {
        ManifestFormat::AgentPlugins => {
            let mut digest = Sha256::new();
            digest.update(id.marketplace_name.as_bytes());
            digest.update([0]);
            digest.update(id.plugin_name.as_bytes());
            data.join(AGENT_PLUGINS_DATA_DIR)
                .join(hex::encode(digest.finalize()))
        }
        _ => data.join(format!("{}-{}", id.plugin_name, id.marketplace_name)),
    }
}

fn directory_names(path: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    let mut names = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Removes `path`, a directory or a file; whether there was one.
pub(crate) fn remove_existing(path: &Path) -> Result<bool, PackageError> {
    let removed = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => Err(error),
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    };
    removed
        .map(|()| true)
        .map_err(|error| io_error(&format!("remove {}", path.display()), error))
}

pub(crate) fn io_error(context: &str, error: std::io::Error) -> PackageError {
    PackageError::Io(format!("failed to {context}: {error}"))
}

pub(crate) fn validate_name_of_source(name: &str) -> Result<(), PackageError> {
    validate_plugin_name(name).map_err(|error| PackageError::Invalid(error.to_string()))
}
