// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/marketplace.rs` (a marketplace's manifest and
// its supported locations, loading and listing marketplaces, finding an
// installable entry) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the marketplaces listed are those whose
// roots the caller names (no discovery from the home directory or the
// repository); an entry is read on its own, so an entry without a name or
// a source is skipped with an issue rather than failing the file; the
// marketplace's name must be one a plugin id can carry; npm sources, the
// `products` and `authentication` policies are not ported.

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value};

use crate::id::validate_marketplace_name;
use crate::manifest::{ManifestInterface, PackageManifest};
use crate::{PackageError, PluginId};

mod entry;
mod manage;
mod source;

pub use entry::{ManifestFallback, PackageSource};
pub use manage::{MarketplaceAddRequest, MarketplaceAdded, MarketplaceUpgrade};

const MANIFEST_PATHS: [&str; 4] = [
    ".agents/plugins/marketplace.json",
    ".agents/plugins/api_marketplace.json",
    ".claude-plugin/marketplace.json",
    ".cursor-plugin/marketplace.json",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Marketplace {
    pub name: String,
    pub root: PathBuf,
    /// The manifest file.
    pub path: PathBuf,
    pub display_name: Option<String>,
    pub plugins: Vec<MarketplacePlugin>,
    /// The entries skipped, each with why.
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarketplacePlugin {
    pub id: PluginId,
    pub source: PackageSource,
    pub installation: InstallationPolicy,
    pub interface: Option<ManifestInterface>,
    pub keywords: Vec<String>,
    /// The version of a local source's manifest.
    pub local_version: Option<String>,
    /// The manifest the entry describes, written into a package that has
    /// none of its own when it is installed.
    pub fallback: ManifestFallback,
    /// The package's manifest: a local source's own, else the fallback's.
    pub manifest: Option<PackageManifest>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InstallationPolicy {
    NotAvailable,
    #[default]
    Available,
    InstalledByDefault,
}

/// The marketplaces at some roots: the ones that loaded, and why each other
/// one did not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarketplaceListing {
    pub marketplaces: Vec<Marketplace>,
    pub errors: Vec<(PathBuf, String)>,
}

/// The marketplace manifest under `root`, probed in Codex's order.
pub fn find_marketplace_manifest(root: &Path) -> Option<PathBuf> {
    MANIFEST_PATHS
        .iter()
        .map(|relative| root.join(relative))
        .find(|path| path.is_file())
}

/// The root of the marketplace whose manifest is `manifest`.
pub(crate) fn marketplace_root(manifest: &Path) -> Option<PathBuf> {
    MANIFEST_PATHS.iter().find_map(|relative| {
        let mut current = manifest;
        for component in Path::new(relative).components().rev() {
            let Component::Normal(expected) = component else {
                return None;
            };
            if current.file_name() != Some(expected) {
                return None;
            }
            current = current.parent()?;
        }
        Some(current.to_path_buf())
    })
}

/// The marketplaces at `roots`, each once; one that does not load is an
/// error of its own and the others are listed.
pub fn list_marketplaces(roots: &[PathBuf]) -> MarketplaceListing {
    let mut listing = MarketplaceListing::default();
    let mut seen = Vec::new();
    for root in roots {
        let Some(path) = find_marketplace_manifest(root) else {
            listing.errors.push((
                root.clone(),
                "the directory holds no marketplace manifest".to_string(),
            ));
            continue;
        };
        if seen.contains(&path) {
            continue;
        }
        seen.push(path.clone());
        match load_marketplace(&path) {
            Ok(marketplace) => listing.marketplaces.push(marketplace),
            Err(error) => listing.errors.push((path, error.to_string())),
        }
    }
    listing
}

/// The marketplace at its manifest `path`. Only a file that is not a JSON
/// object with a valid `name` and a `plugins` list fails; an entry that
/// cannot be used is skipped with an issue.
pub fn load_marketplace(path: &Path) -> Result<Marketplace, PackageError> {
    let invalid = |message: String| {
        PackageError::Invalid(format!(
            "invalid marketplace file `{}`: {message}",
            path.display()
        ))
    };
    let root = marketplace_root(path)
        .ok_or_else(|| invalid("the file is not in a supported location".to_string()))?;
    let contents = fs::read_to_string(path).map_err(|error| match error.kind() {
        ErrorKind::NotFound => PackageError::NotFound(format!(
            "marketplace file `{}` does not exist",
            path.display()
        )),
        _ => PackageError::Io(format!("failed to read {}: {error}", path.display())),
    })?;
    let object = match serde_json::from_str::<Value>(&contents) {
        Ok(Value::Object(object)) => object,
        Ok(_) => return Err(invalid("expected a JSON object".to_string())),
        Err(error) => return Err(invalid(error.to_string())),
    };
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("the marketplace has no name".to_string()))?
        .to_string();
    validate_marketplace_name(&name).map_err(|error| invalid(error.to_string()))?;
    let entries = object
        .get("plugins")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("the marketplace has no plugins list".to_string()))?;
    let mut marketplace = Marketplace {
        display_name: display_name(&object),
        name,
        root,
        path: path.to_path_buf(),
        plugins: Vec::new(),
        issues: Vec::new(),
    };
    for (index, value) in entries.iter().enumerate() {
        match entry::resolve(&marketplace, value) {
            Ok(plugin) => marketplace.plugins.push(plugin),
            Err(message) => marketplace
                .issues
                .push(format!("skipping plugins[{index}]: {message}")),
        }
    }
    Ok(marketplace)
}

fn display_name(object: &Map<String, Value>) -> Option<String> {
    object
        .get("interface")?
        .get("displayName")?
        .as_str()
        .map(str::to_string)
}

impl Marketplace {
    /// The first entry named `plugin_name` that can be installed.
    pub fn installable(&self, plugin_name: &str) -> Result<&MarketplacePlugin, PackageError> {
        let plugin = self
            .plugins
            .iter()
            .find(|plugin| plugin.id.plugin_name == plugin_name)
            .ok_or_else(|| {
                PackageError::NotFound(format!(
                    "plugin `{plugin_name}` was not found in marketplace `{}`",
                    self.name
                ))
            })?;
        if plugin.installation == InstallationPolicy::NotAvailable {
            return Err(PackageError::Invalid(format!(
                "plugin `{plugin_name}` is not available for install in marketplace `{}`",
                self.name
            )));
        }
        Ok(plugin)
    }
}
