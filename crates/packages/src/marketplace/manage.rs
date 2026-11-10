// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/marketplace_add.rs` (with
// `marketplace_add/install.rs` and `marketplace_add/metadata.rs`),
// `codex-rs/core-plugins/src/marketplace_upgrade.rs` (with
// `marketplace_upgrade/activation.rs`),
// `codex-rs/core-plugins/src/marketplace_remove.rs` and
// `codex-rs/core-plugins/src/installed_marketplaces.rs` (adding a
// marketplace from a directory or a git clone, upgrading a git one when its
// remote moved, removing one, the root of each) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the store does not write the
// configuration: an add or an upgrade returns the `[marketplaces]` record
// for the caller to write, and an add and a remove are given the records
// configured; the revision checked out is the record's `lastRevision`,
// with no metadata file in the checkout; admin source policies, automatic
// upgrades and their in-flight checks are not ported.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;

use super::source::{parse_add_source, AddSource};
use super::{find_marketplace_manifest, list_marketplaces, load_marketplace, MarketplaceListing};
use crate::git::{clone, remote_revision, CloneRequest};
use crate::id::validate_marketplace_name;
use crate::store::{io_error, remove_existing};
use crate::{MarketplaceRecord, MarketplaceSourceType, PackageError, PackageStore};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarketplaceAddRequest {
    /// A local directory, a git URL (`#ref` allowed) or `owner/repo[@ref]`.
    pub source: String,
    pub ref_name: Option<String>,
    /// For a git source, the paths to check out alone.
    pub sparse_paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketplaceAdded {
    pub name: String,
    /// What to write in `[marketplaces.<name>]`.
    pub record: MarketplaceRecord,
    pub root: PathBuf,
    /// The same source was added before; nothing changed.
    pub already_added: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarketplaceUpgrade {
    /// A local marketplace, or a git one already at its remote's revision.
    UpToDate,
    /// The checkout was replaced; the record to write.
    Upgraded(MarketplaceRecord),
}

impl PackageStore {
    /// The root of the marketplace `name` the user added.
    pub fn marketplace_root(&self, name: &str, record: &MarketplaceRecord) -> PathBuf {
        match record.source_type {
            MarketplaceSourceType::Local => PathBuf::from(&record.source),
            MarketplaceSourceType::Git => self.marketplace_checkout(name),
        }
    }

    /// The marketplaces the user added, as `records` names them.
    pub fn configured_marketplaces(
        &self,
        records: &BTreeMap<String, MarketplaceRecord>,
    ) -> MarketplaceListing {
        let roots = records
            .iter()
            .map(|(name, record)| self.marketplace_root(name, record))
            .collect::<Vec<_>>();
        list_marketplaces(&roots)
    }

    /// Adds the marketplace at `request.source`: a local directory is used
    /// where it is, a git repository is cloned under `marketplaces/`.
    /// `configured` is `[marketplaces]` as it stands.
    pub fn add_marketplace(
        &self,
        request: &MarketplaceAddRequest,
        configured: &BTreeMap<String, MarketplaceRecord>,
    ) -> Result<MarketplaceAdded, PackageError> {
        let source = parse_add_source(&request.source, request.ref_name.clone())?;
        if !request.sparse_paths.is_empty() && !matches!(source, AddSource::Git { .. }) {
            return Err(PackageError::Invalid(
                "--sparse is only supported for git marketplace sources".to_string(),
            ));
        }
        let _change = self.changes();
        if let Some(added) = self.already_added(&source, &request.sparse_paths, configured) {
            return Ok(added);
        }
        match source {
            AddSource::Local { path } => {
                let name = validated_name(path.system())?;
                self.ensure_name_is_free(&name, configured)?;
                Ok(MarketplaceAdded {
                    record: MarketplaceRecord {
                        source_type: MarketplaceSourceType::Local,
                        source: path.native_text().to_string(),
                        ref_name: None,
                        sparse_paths: None,
                        last_updated: Some(now()),
                        last_revision: None,
                        enabled: None,
                    },
                    name,
                    root: path.native().to_path_buf(),
                    already_added: false,
                })
            }
            AddSource::Git { url, ref_name } => {
                let mut record = MarketplaceRecord {
                    source_type: MarketplaceSourceType::Git,
                    source: url,
                    ref_name,
                    sparse_paths: (!request.sparse_paths.is_empty())
                        .then(|| request.sparse_paths.clone()),
                    last_updated: None,
                    last_revision: None,
                    enabled: None,
                };
                let (name, revision) = self.add_git(&record, configured)?;
                record.last_updated = Some(now());
                record.last_revision = Some(revision);
                Ok(MarketplaceAdded {
                    root: self.marketplace_checkout(&name),
                    name,
                    record,
                    already_added: false,
                })
            }
        }
    }

    /// The marketplace configured from the same source, when its root is
    /// still a marketplace.
    fn already_added(
        &self,
        source: &AddSource,
        sparse_paths: &[String],
        configured: &BTreeMap<String, MarketplaceRecord>,
    ) -> Option<MarketplaceAdded> {
        configured.iter().find_map(|(name, record)| {
            let same = match source {
                AddSource::Local { path } => {
                    record.source_type == MarketplaceSourceType::Local
                        && ResolvedPath::resolve(&record.source)
                            .is_ok_and(|configured| configured.key() == path.key())
                }
                AddSource::Git { url, ref_name } => {
                    record.source_type == MarketplaceSourceType::Git
                        && &record.source == url
                        && &record.ref_name == ref_name
                        && record.sparse_paths.as_deref().unwrap_or_default() == sparse_paths
                }
            };
            let root = self.marketplace_root(name, record);
            (same && validated_name(&root).is_ok()).then(|| MarketplaceAdded {
                name: name.clone(),
                record: record.clone(),
                root,
                already_added: true,
            })
        })
    }

    fn ensure_name_is_free(
        &self,
        name: &str,
        configured: &BTreeMap<String, MarketplaceRecord>,
    ) -> Result<(), PackageError> {
        let taken = configured
            .get(name)
            .is_some_and(|record| validated_name(&self.marketplace_root(name, record)).is_ok())
            || self.marketplace_checkout(name).exists();
        if taken {
            return Err(PackageError::Invalid(format!(
                "marketplace '{name}' is already added from a different source; remove it before adding this source"
            )));
        }
        Ok(())
    }

    fn add_git(
        &self,
        record: &MarketplaceRecord,
        configured: &BTreeMap<String, MarketplaceRecord>,
    ) -> Result<(String, String), PackageError> {
        let staging = self.staging("marketplace-")?;
        let checkout = staging.path().join("checkout");
        let revision = self.clone_marketplace(record, &checkout)?;
        let name = validated_name(&checkout)?;
        self.ensure_name_is_free(&name, configured)?;
        let destination = self.marketplace_checkout(&name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_error("create the marketplaces directory", error))?;
        }
        fs::rename(&checkout, &destination)
            .map_err(|error| io_error("install the marketplace", error))?;
        Ok((name, revision))
    }

    fn clone_marketplace(
        &self,
        record: &MarketplaceRecord,
        destination: &Path,
    ) -> Result<String, PackageError> {
        let request = CloneRequest {
            url: &record.source,
            sparse_paths: record.sparse_paths.as_deref().unwrap_or_default(),
            cone: true,
            ref_name: record.ref_name.as_deref(),
            sha: None,
        };
        clone(self.executor(), &request, destination).map_err(PackageError::Git)
    }

    /// Brings the git marketplace `name` to its remote's revision: a fresh
    /// clone replaces the checkout, the previous one kept until the swap
    /// succeeded. The clone must still be the marketplace `name`.
    pub fn upgrade_marketplace(
        &self,
        name: &str,
        record: &MarketplaceRecord,
    ) -> Result<MarketplaceUpgrade, PackageError> {
        validate_marketplace_name(name)
            .map_err(|error| PackageError::Invalid(error.to_string()))?;
        if record.source_type == MarketplaceSourceType::Local {
            return Ok(MarketplaceUpgrade::UpToDate);
        }
        let _change = self.changes();
        let destination = self.marketplace_checkout(name);
        let cwd = self.staging("ls-remote-")?;
        let remote = remote_revision(
            self.executor(),
            cwd.path(),
            &record.source,
            record.ref_name.as_deref(),
        )
        .map_err(PackageError::Git)?;
        let current = validated_name(&destination).is_ok_and(|found| found == name);
        if current && record.last_revision.as_deref() == Some(remote.as_str()) {
            return Ok(MarketplaceUpgrade::UpToDate);
        }
        let staging = self.staging("marketplace-")?;
        let checkout = staging.path().join("checkout");
        let revision = self.clone_marketplace(record, &checkout)?;
        let found = validated_name(&checkout)?;
        if found != name {
            return Err(PackageError::Invalid(format!(
                "upgraded marketplace name `{found}` does not match configured marketplace `{name}`"
            )));
        }
        replace_checkout(&checkout, &destination, &staging.path().join("previous"))?;
        Ok(MarketplaceUpgrade::Upgraded(MarketplaceRecord {
            last_updated: Some(now()),
            last_revision: Some(revision),
            ..record.clone()
        }))
    }

    /// Removes the checkout of the marketplace `name`, when it has one;
    /// the caller removes its record. A local marketplace's directory is
    /// the user's and is left.
    pub fn remove_marketplace(
        &self,
        name: &str,
        record: Option<&MarketplaceRecord>,
    ) -> Result<Option<PathBuf>, PackageError> {
        validate_marketplace_name(name)
            .map_err(|error| PackageError::Invalid(error.to_string()))?;
        let _change = self.changes();
        let checkout = self.marketplace_checkout(name);
        let removed = remove_existing(&checkout)?;
        if !removed && record.is_none() {
            return Err(PackageError::NotFound(format!(
                "marketplace `{name}` is not configured or installed"
            )));
        }
        Ok(removed.then_some(checkout))
    }
}

/// The name of the marketplace at `root`, which must be one a plugin id
/// can carry.
fn validated_name(root: &Path) -> Result<String, PackageError> {
    let manifest = find_marketplace_manifest(root).ok_or_else(|| {
        PackageError::Invalid(format!(
            "invalid marketplace file `{}`: marketplace root does not contain a supported manifest",
            root.display()
        ))
    })?;
    load_marketplace(&manifest).map(|marketplace| marketplace.name)
}

fn replace_checkout(staged: &Path, destination: &Path, backup: &Path) -> Result<(), PackageError> {
    if !destination.exists() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_error("create the marketplaces directory", error))?;
        }
        return fs::rename(staged, destination)
            .map_err(|error| io_error("activate the upgraded marketplace", error));
    }
    fs::rename(destination, backup)
        .map_err(|error| io_error("move the previous marketplace out of the way", error))?;
    if let Err(error) = fs::rename(staged, destination) {
        let restored = fs::rename(backup, destination);
        return Err(PackageError::Io(match restored {
            Ok(()) => format!("failed to activate the upgraded marketplace: {error}"),
            Err(rollback) => format!(
                "failed to activate the upgraded marketplace at {}: {error}; failed to restore the previous one: {rollback}",
                destination.display()
            ),
        }));
    }
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
