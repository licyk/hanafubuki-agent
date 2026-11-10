//! Which installed packages are on: `[plugins]` with `_default`, and a
//! marketplace turned off.

use std::collections::BTreeMap;

use hanafubuki_agent_packages::{
    is_package_enabled, MarketplaceRecord, MarketplaceSourceType, PluginDefaults, PluginId,
    PluginSettings, PluginsSettings,
};

fn record(enabled: Option<bool>) -> MarketplaceRecord {
    MarketplaceRecord {
        source_type: MarketplaceSourceType::Git,
        source: "https://example.com/m.git".to_string(),
        ref_name: None,
        sparse_paths: None,
        last_updated: None,
        last_revision: None,
        enabled,
    }
}

fn entry(enabled: Option<bool>) -> PluginSettings {
    PluginSettings {
        enabled,
        ..PluginSettings::default()
    }
}

#[test]
fn an_entry_overrides_the_default_which_overrides_on() {
    let id = PluginId::parse("p@m").unwrap();
    let none = BTreeMap::new();
    let mut plugins = PluginsSettings::default();
    assert!(is_package_enabled(&id, &plugins, &none));

    plugins.default = Some(PluginDefaults { enabled: false });
    assert!(!is_package_enabled(&id, &plugins, &none));

    plugins
        .packages
        .insert("p@m".to_string(), entry(Some(true)));
    assert!(is_package_enabled(&id, &plugins, &none));

    plugins.packages.insert("p@m".to_string(), entry(None));
    assert!(!is_package_enabled(&id, &plugins, &none));
}

#[test]
fn a_marketplace_turned_off_turns_its_packages_off_whatever_plugins_says() {
    let id = PluginId::parse("p@m").unwrap();
    let mut plugins = PluginsSettings {
        default: Some(PluginDefaults { enabled: true }),
        ..PluginsSettings::default()
    };
    plugins
        .packages
        .insert("p@m".to_string(), entry(Some(true)));

    let off = BTreeMap::from([("m".to_string(), record(Some(false)))]);
    let on = BTreeMap::from([("m".to_string(), record(None))]);
    assert!(!is_package_enabled(&id, &plugins, &off));
    assert!(is_package_enabled(&id, &plugins, &on));
}
