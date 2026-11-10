//! A plugin's directory checked before it is installed (`plugins
//! validate`): its manifest read as an install reads it, the files it
//! declares, and, for a package with code, what the guest host would refuse
//! or leave out. Nothing is installed and nothing starts.

use std::path::{Path, PathBuf};

use hanafubuki_agent_packages::{
    load_manifest, HooksDeclaration, ManifestFormat, McpServersDeclaration, PackageManifest,
    PluginId,
};
use hanafubuki_agent_plugin_packages::check_package;
use serde::Serialize;

use super::LOCAL_MARKETPLACE;

/// A plugin's directory as `plugins validate` found it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPackageValidation {
    /// Its manifest's name: the plugin part of its id.
    pub name: String,
    pub version: Option<String>,
    pub format: ManifestFormat,
    /// The manifest that was read.
    pub manifest: PathBuf,
    /// Whether it has code (a `runtime`, `contributes`, `permissions` or
    /// `configSchema`), which runs as a guest.
    pub code: bool,
    /// Everything an install would leave out, or that would keep its code
    /// from starting or a declaration of it from registering; none when it
    /// is valid.
    pub issues: Vec<String>,
}

/// Checks the plugin in `root`. An error is a directory with no manifest an
/// install accepts, or one that cannot be read at all.
pub fn validate_package(root: &Path) -> Result<AgentPackageValidation, String> {
    let loaded = load_manifest(root).map_err(|error| error.to_string())?;
    let mut issues = loaded.issues;
    issues.extend(missing_paths(&loaded.manifest));
    let code = loaded.manifest.code.as_ref();
    if let Some(code) = code {
        match PluginId::new(&loaded.manifest.name, LOCAL_MARKETPLACE) {
            Ok(id) => issues.extend(check_package(&id, root, code)),
            Err(error) => issues.push(error.to_string()),
        }
    }
    Ok(AgentPackageValidation {
        name: loaded.manifest.name.clone(),
        version: loaded.manifest.version.clone(),
        format: loaded.format,
        manifest: loaded.path,
        code: code.is_some(),
        issues,
    })
}

/// The files and directories the manifest declares that are not there.
/// (Undeclared components fall back to their default places, which may be
/// absent.)
fn missing_paths(manifest: &PackageManifest) -> Vec<String> {
    let paths = &manifest.paths;
    let mut declared: Vec<(&str, &Path)> = paths
        .skills
        .iter()
        .map(|path| ("skills", path.as_path()))
        .collect();
    if let Some(McpServersDeclaration::Path(path)) = &paths.mcp_servers {
        declared.push(("mcpServers", path));
    }
    if let Some(HooksDeclaration::Paths(hooks)) = &paths.hooks {
        declared.extend(hooks.iter().map(|path| ("hooks", path.as_path())));
    }
    declared
        .into_iter()
        .filter(|(_, path)| !path.exists())
        .map(|(field, path)| format!("{field}: {} does not exist", path.display()))
        .collect()
}
