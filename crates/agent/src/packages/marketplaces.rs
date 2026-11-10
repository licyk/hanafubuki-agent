//! The marketplaces the user added, read for the facade: each one's
//! manifest from its directory or checkout, its plugins marked installed
//! and on as the store and the settings have them.

use hanafubuki_agent_config::AgentSettings;
use hanafubuki_agent_packages::{
    find_marketplace_manifest, load_marketplace, InstallationPolicy, InstalledPackage, Marketplace,
    MarketplaceRecord, PackageError, PackageStore,
};

use super::lifecycle::{AgentMarketplace, AgentMarketplacePlugin};

/// The added marketplace `name`, read from its root.
pub(crate) fn configured_marketplace(
    store: &PackageStore,
    name: &str,
    record: &MarketplaceRecord,
) -> Result<Marketplace, PackageError> {
    let root = store.marketplace_root(name, record);
    let manifest = find_marketplace_manifest(&root).ok_or_else(|| {
        PackageError::NotFound(format!(
            "marketplace `{name}` has no marketplace manifest at {}",
            root.display()
        ))
    })?;
    load_marketplace(&manifest)
}

/// The added marketplace `name` as the facade reports it; one that cannot
/// be read carries why.
pub(crate) fn marketplace_view(
    store: &PackageStore,
    settings: &AgentSettings,
    installed: &[InstalledPackage],
    name: &str,
    record: &MarketplaceRecord,
) -> AgentMarketplace {
    let mut view = AgentMarketplace {
        name: name.to_string(),
        display_name: None,
        source_type: record.source_type,
        source: record.source.clone(),
        ref_name: record.ref_name.clone(),
        sparse_paths: record.sparse_paths.clone().unwrap_or_default(),
        enabled: record.enabled != Some(false),
        last_updated: record.last_updated.clone(),
        last_revision: record.last_revision.clone(),
        root: store.marketplace_root(name, record),
        plugins: Vec::new(),
        issues: Vec::new(),
        error: None,
    };
    let marketplace = match configured_marketplace(store, name, record) {
        Ok(marketplace) => marketplace,
        Err(error) => {
            view.error = Some(error.to_string());
            return view;
        }
    };
    view.display_name = marketplace.display_name;
    view.issues = marketplace.issues;
    view.plugins = marketplace
        .plugins
        .into_iter()
        .map(|plugin| {
            let installed = installed.iter().any(|package| package.id == plugin.id);
            let manifest = plugin.manifest.as_ref();
            AgentMarketplacePlugin {
                id: plugin.id.key(),
                display_name: manifest.map_or_else(
                    || plugin.id.plugin_name.clone(),
                    |manifest| manifest.display_name().to_string(),
                ),
                description: manifest.and_then(|manifest| manifest.description.clone()),
                version: plugin.local_version,
                installable: plugin.installation != InstallationPolicy::NotAvailable,
                enabled: installed && settings.package_enabled(&plugin.id),
                installed,
                name: plugin.id.plugin_name,
            }
        })
        .collect();
    view
}
