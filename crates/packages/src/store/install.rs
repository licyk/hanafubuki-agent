// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/store.rs` (an install: the fallback manifest
// only when the source has none, the version directory from the manifest,
// the manifest's name against the entry's) and the source materialization
// of `codex-rs/core-plugins/src/loader.rs` and the install of
// `codex-rs/core-plugins/src/manager.rs`, at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the clone of a git source runs through the
// Agent's `ExecutorManager`; the install takes the package's content hash
// and records it with the version and the source in `install.json`; the
// curated-marketplace version and npm sources are not ported.

use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::stage::{replace_root, StagedManifest};
use super::{io_error, validate_name_of_source, InstallRecord, PackageStore};
use crate::git::{clone, CloneRequest};
use crate::manifest::{
    agent_plugins_schema_status, find_manifest, load_manifest, parse_manifest, ManifestFormat,
    SchemaStatus,
};
use crate::version::{validate_version_segment, LOCAL_VERSION};
use crate::{content_hash, Marketplace, PackageError, PackageSource, PluginId};

/// The version directory of an Agent Plugins package whose manifest names
/// no version.
const DEFAULT_AGENT_PLUGINS_VERSION: &str = "1.0.0";

/// What an install did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallOutcome {
    pub id: PluginId,
    pub version: String,
    pub root: PathBuf,
    pub content_hash: String,
    /// The Skills the package's commands became.
    pub migrated_commands: Vec<String>,
    /// What its manifest's reading left out.
    pub issues: Vec<String>,
}

impl PackageStore {
    /// Installs the package directory `source` as `id`; its manifest's name
    /// must be `id`'s plugin name.
    pub fn install_directory(
        &self,
        source: &Path,
        id: &PluginId,
    ) -> Result<InstallOutcome, PackageError> {
        let resolved = ResolvedPath::resolve(source).map_err(|error| {
            PackageError::Invalid(format!("plugin source path cannot be used: {error}"))
        })?;
        let recorded = PackageSource::Local {
            path: resolved.native().to_path_buf(),
        };
        let _change = self.changes();
        self.install(resolved.system(), id, None, recorded)
    }

    /// Installs the entry `plugin_name` of `marketplace`, fetching a git
    /// source first.
    pub fn install_from_marketplace(
        &self,
        marketplace: &Marketplace,
        plugin_name: &str,
    ) -> Result<InstallOutcome, PackageError> {
        let entry = marketplace.installable(plugin_name)?;
        let fallback = entry
            .fallback
            .has_metadata
            .then_some(entry.fallback.contents.as_str());
        let _change = self.changes();
        match &entry.source {
            PackageSource::Local { path } => {
                self.install(path, &entry.id, fallback, entry.source.clone())
            }
            PackageSource::Git {
                url,
                path,
                ref_name,
                sha,
            } => {
                let staging = self.staging("source-")?;
                let checkout = staging.path().join("checkout");
                let sparse_paths = path.iter().cloned().collect::<Vec<_>>();
                let request = CloneRequest {
                    url,
                    sparse_paths: &sparse_paths,
                    cone: false,
                    ref_name: ref_name.as_deref(),
                    sha: sha.as_deref(),
                };
                clone(self.executor(), &request, &checkout).map_err(PackageError::Git)?;
                let source = path
                    .as_ref()
                    .map_or(checkout.clone(), |path| checkout.join(path));
                self.install(&source, &entry.id, fallback, entry.source.clone())
            }
        }
    }

    fn install(
        &self,
        source: &Path,
        id: &PluginId,
        fallback: Option<&str>,
        recorded: PackageSource,
    ) -> Result<InstallOutcome, PackageError> {
        if !fs::metadata(source).is_ok_and(|metadata| metadata.is_dir()) {
            return Err(PackageError::Invalid(format!(
                "plugin source path is not a directory: {}",
                source.display()
            )));
        }
        // A manifest of the package's own always wins over the entry's.
        let manifest = match find_manifest(source) {
            Some((_, path)) => StagedManifest::OnDisk(path),
            None => match fallback {
                Some(contents) => StagedManifest::Fallback(contents),
                None => return Err(PackageError::Invalid("missing plugin.json".to_string())),
            },
        };
        let contents = manifest_contents(&manifest)?;
        let version = version_for(&contents)?;
        let (parsed, mut issues) = match &manifest {
            StagedManifest::OnDisk(_) => load_manifest(source)
                .map(|loaded| (loaded.manifest, loaded.issues))
                .map_err(|error| PackageError::Invalid(error.to_string()))?,
            StagedManifest::Fallback(contents) => {
                parse_manifest(source, ManifestFormat::Codex, contents)
                    .map_err(|error| PackageError::Invalid(error.to_string()))?
            }
        };
        validate_name_of_source(&parsed.name)?;
        if parsed.name != id.plugin_name {
            return Err(PackageError::Invalid(format!(
                "plugin.json name `{}` does not match marketplace plugin name `{}`",
                parsed.name, id.plugin_name
            )));
        }
        let staged = replace_root(
            self,
            source,
            &self.package_base(id),
            &version,
            &manifest,
            &parsed.paths.commands,
        )?;
        issues.extend(staged.issues);
        let root = self.package_root(id, &version);
        let content_hash =
            content_hash(&root).map_err(|error| io_error("hash the installed plugin", error))?;
        self.update_record(|records| {
            records.insert(
                id.clone(),
                InstallRecord {
                    version: Some(version.clone()),
                    format: Some(staged.format),
                    source: Some(recorded),
                    content_hash: Some(content_hash.clone()),
                },
            );
        })?;
        log::info!("plugin {id} installed version={version}");
        Ok(InstallOutcome {
            id: id.clone(),
            version,
            root,
            content_hash,
            migrated_commands: staged.migrated_commands,
            issues,
        })
    }
}

fn manifest_contents(manifest: &StagedManifest<'_>) -> Result<String, PackageError> {
    match manifest {
        StagedManifest::OnDisk(path) => {
            fs::read_to_string(path).map_err(|error| io_error("read plugin.json", error))
        }
        StagedManifest::Fallback(contents) => Ok(contents.to_string()),
    }
}

/// The version directory: the manifest's `version`, else `local`; an Agent
/// Plugins package's blank or unusable version is `1.0.0` or a digest of
/// it, where any other package's fails the install.
fn version_for(contents: &str) -> Result<String, PackageError> {
    let invalid = |message: &str| {
        PackageError::Invalid(format!("invalid plugin version in plugin.json: {message}"))
    };
    let agent_plugins = agent_plugins_schema_status(contents) == SchemaStatus::Supported;
    let value = serde_json::from_str::<Value>(contents)
        .map_err(|error| PackageError::Invalid(format!("failed to parse plugin.json: {error}")))?;
    let version = match value.get("version") {
        None | Some(Value::Null) => None,
        Some(Value::String(version)) => Some(version.trim().to_string()),
        Some(_) => return Err(invalid("expected string")),
    };
    let version = match version {
        Some(version) if version.is_empty() && !agent_plugins => {
            return Err(invalid("must not be blank"))
        }
        Some(version) if !version.is_empty() => version,
        _ if agent_plugins => DEFAULT_AGENT_PLUGINS_VERSION.to_string(),
        _ => LOCAL_VERSION.to_string(),
    };
    match validate_version_segment(&version) {
        Ok(()) => Ok(version),
        Err(_) if agent_plugins => {
            let digest = Sha256::digest(version.as_bytes());
            Ok(format!("agent-plugins-{}", hex::encode(&digest[..12])))
        }
        Err(message) => Err(PackageError::Invalid(message)),
    }
}
