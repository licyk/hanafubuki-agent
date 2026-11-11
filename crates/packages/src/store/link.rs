//! A package linked for development (`plugins link`): the install record
//! names its directory, which is the package itself. Nothing is copied
//! into the cache, no content hash pins it, and nothing is written into the
//! directory, so its `commands/` are not turned into Skills.

use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;

use super::{validate_name_of_source, InstallRecord, PackageStore};
use crate::commands::declares_commands;
use crate::manifest::load_manifest;
use crate::{PackageError, PackageSource, PluginId};

/// What a link did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkOutcome {
    pub id: PluginId,
    /// The linked directory, as a person reads it.
    pub root: PathBuf,
    /// What its manifest's reading left out, and what a link does not do.
    pub issues: Vec<String>,
}

impl PackageStore {
    /// Links the package directory `source` as `id`; its manifest's name
    /// must be `id`'s plugin name. A package installed in the cache as `id`
    /// is not replaced. Linking `id` again keeps what was granted to its
    /// code, as an install does.
    pub fn link_directory(
        &self,
        source: &Path,
        id: &PluginId,
    ) -> Result<LinkOutcome, PackageError> {
        let directory = ResolvedPath::resolve(source).map_err(|error| {
            PackageError::Invalid(format!("plugin source path cannot be used: {error}"))
        })?;
        if !directory.system().is_dir() {
            return Err(PackageError::Invalid(format!(
                "plugin source path is not a directory: {}",
                directory.native().display()
            )));
        }
        let loaded = load_manifest(directory.system())
            .map_err(|error| PackageError::Invalid(error.to_string()))?;
        validate_name_of_source(&loaded.manifest.name)?;
        if loaded.manifest.name != id.plugin_name {
            return Err(PackageError::Invalid(format!(
                "plugin.json name `{}` does not match plugin name `{}`",
                loaded.manifest.name, id.plugin_name
            )));
        }
        let _change = self.changes();
        if self.active_version(id).is_some() {
            return Err(PackageError::Invalid(format!(
                "plugin `{id}` is installed; remove it before linking a directory as it"
            )));
        }
        let mut issues = loaded.issues;
        if declares_commands(directory.system(), &loaded.manifest.paths.commands) {
            issues.push(
                "its commands are not turned into Skills while it is linked; an install turns them"
                    .to_string(),
            );
        }
        let root = directory.native().to_path_buf();
        self.update_record(|records| {
            let granted_permissions = records
                .get(id)
                .and_then(|previous| previous.granted_permissions.clone());
            records.insert(
                id.clone(),
                InstallRecord {
                    version: loaded.manifest.version.clone(),
                    format: Some(loaded.format),
                    source: Some(PackageSource::Linked { path: root.clone() }),
                    content_hash: None,
                    granted_permissions,
                },
            );
        })?;
        log::info!("plugin {id} linked");
        Ok(LinkOutcome {
            id: id.clone(),
            root,
            issues,
        })
    }
}
