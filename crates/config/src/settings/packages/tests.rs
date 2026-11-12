use std::fs;

use hanafubuki_agent_packages::{
    MarketplaceRecord, MarketplaceSourceType, PluginDefaults, PluginId, PluginMcpServerSettings,
    PluginSettings,
};
use serde_json::{json, Map, Value};

use crate::{
    read_settings_document, AgentConfig, ConfigLayers, ConfigOverrides, AGENT_SETTINGS_FILE,
};

fn document(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        _ => unreachable!(),
    }
}

fn git_record(source: &str) -> MarketplaceRecord {
    MarketplaceRecord {
        source_type: MarketplaceSourceType::Git,
        source: source.to_string(),
        ref_name: Some("main".to_string()),
        sparse_paths: Some(vec!["plugins".to_string()]),
        last_updated: Some("2026-11-10T00:00:00Z".to_string()),
        last_revision: Some("abc".to_string()),
        enabled: None,
    }
}

#[test]
fn plugins_and_marketplaces_read_field_by_field() {
    let (settings, repairs) = read_settings_document(&document(json!({
        "plugins": {
            "_default": {"enabled": false},
            "demo@my-mkt": {
                "enabled": true,
                "mcpServers": {"server1": {"enabledTools": ["a"], "disabledTools": "b"}},
                "settings": {"board": "B-1", "limit": 3}
            },
            "broken@my-mkt": {"settings": "everything"},
            "not an id": {"enabled": true},
            "other@my-mkt": "on"
        },
        "marketplaces": {
            "my-mkt": {"sourceType": "git", "source": "https://example.com/m.git", "ref": "main",
                       "sparsePaths": ["plugins"], "lastRevision": "abc", "enabled": "no"},
            "local": {"sourceType": "local", "source": "/srv/market"},
            "sourceless": {"sourceType": "git"},
            "dotted.name": {"sourceType": "local", "source": "/x"}
        }
    })));

    assert_eq!(
        settings.plugins.default,
        Some(PluginDefaults { enabled: false })
    );
    assert_eq!(
        settings.plugins.packages.keys().collect::<Vec<_>>(),
        vec!["broken@my-mkt", "demo@my-mkt"]
    );
    let demo = &settings.plugins.packages["demo@my-mkt"];
    assert_eq!(demo.enabled, Some(true));
    assert_eq!(
        demo.settings
            .as_ref()
            .map(|settings| Value::Object(settings.clone())),
        Some(json!({"board": "B-1", "limit": 3}))
    );
    assert_eq!(settings.plugins.packages["broken@my-mkt"].settings, None);
    assert_eq!(
        demo.mcp_servers["server1"],
        PluginMcpServerSettings {
            enabled: None,
            enabled_tools: Some(vec!["a".to_string()]),
            disabled_tools: None,
        }
    );
    assert_eq!(
        settings.marketplaces.keys().collect::<Vec<_>>(),
        vec!["local", "my-mkt"]
    );
    let market = &settings.marketplaces["my-mkt"];
    assert_eq!(market.source_type, MarketplaceSourceType::Git);
    assert_eq!(market.ref_name.as_deref(), Some("main"));
    assert_eq!(market.sparse_paths, Some(vec!["plugins".to_string()]));
    assert_eq!(market.enabled, None);
    assert_eq!(
        repairs,
        vec![
            "plugins.broken@my-mkt.settings: the value is invalid and was replaced with its default",
            "plugins.demo@my-mkt.mcpServers.server1.disabledTools: the value is invalid and was replaced with its default",
            "marketplaces.my-mkt.enabled: the value is invalid and was replaced with its default",
        ]
    );
}

#[test]
fn enablement_follows_the_entry_the_default_and_the_marketplace() {
    let (settings, _) = read_settings_document(&document(json!({
        "plugins": {"_default": {"enabled": false}, "on@m": {"enabled": true}, "off-market@x": {"enabled": true}},
        "marketplaces": {"x": {"sourceType": "local", "source": "/x", "enabled": false}}
    })));

    assert!(settings.package_enabled(&PluginId::parse("on@m").unwrap()));
    assert!(!settings.package_enabled(&PluginId::parse("unlisted@m").unwrap()));
    assert!(!settings.package_enabled(&PluginId::parse("off-market@x").unwrap()));
}

#[test]
fn layers_merge_the_tables_entry_by_entry() {
    let root = tempfile::tempdir().unwrap();
    let config_dir = root.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join(AGENT_SETTINGS_FILE),
        r#"
[plugins."a@m"]
enabled = true
[plugins."a@m".mcpServers.s]
enabledTools = ["x"]
[plugins."b@m"]
enabled = true
[marketplaces.m]
sourceType = "git"
source = "https://example.com/m.git"
"#,
    )
    .unwrap();
    let config = AgentConfig::layered(
        &config_dir,
        ConfigLayers {
            overrides: ConfigOverrides::parse(&[
                "plugins.a@m.enabled=false",
                "marketplaces.m.enabled=false",
            ])
            .unwrap(),
            ..ConfigLayers::default()
        },
    );

    let settings = config.settings().unwrap();

    let a = &settings.plugins.packages["a@m"];
    assert_eq!(a.enabled, Some(false));
    assert_eq!(
        a.mcp_servers["s"].enabled_tools,
        Some(vec!["x".to_string()])
    );
    assert_eq!(settings.plugins.packages["b@m"].enabled, Some(true));
    assert_eq!(settings.marketplaces["m"].enabled, Some(false));
    assert_eq!(
        settings.marketplaces["m"].source,
        "https://example.com/m.git"
    );
    assert_eq!(config.layer_warnings().unwrap(), Vec::<String>::new());
}

#[test]
fn package_and_marketplace_entries_are_written_and_removed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_SETTINGS_FILE);
    fs::write(&path, "# mine\npermissionMode = \"manual\"\n").unwrap();
    let config = AgentConfig::new(directory.path());
    let id = PluginId::parse("demo@my-mkt").unwrap();

    config
        .set_marketplace("my-mkt", Some(&git_record("https://example.com/m.git")))
        .unwrap();
    config
        .set_package_settings(
            &id,
            Some(&PluginSettings {
                enabled: Some(true),
                ..PluginSettings::default()
            }),
        )
        .unwrap();
    let board = |board: &str| Map::from_iter([("board".to_string(), json!(board))]);
    let mut first = board("B-0");
    first.insert("limit".to_string(), json!(3));
    config.set_package_code_settings(&id, Some(&first)).unwrap();
    // New settings replace the old, and writing the rest of the entry
    // keeps them.
    config
        .set_package_code_settings(&id, Some(&board("B-1")))
        .unwrap();
    let settings = config
        .set_package_settings(
            &id,
            Some(&PluginSettings {
                enabled: Some(true),
                ..PluginSettings::default()
            }),
        )
        .unwrap();
    assert_eq!(
        settings.plugins.packages[&id.key()].settings,
        Some(board("B-1"))
    );

    assert!(settings.package_enabled(&id));
    let text = fs::read_to_string(&path).unwrap();
    for written in [
        "# mine",
        "[marketplaces.my-mkt]",
        "sourceType = \"git\"",
        "ref = \"main\"",
        "sparsePaths = [\"plugins\"]",
        "[plugins.\"demo@my-mkt\"]",
        "enabled = true",
        "[plugins.\"demo@my-mkt\".settings]",
        "board = \"B-1\"",
    ] {
        assert!(text.contains(written), "{written:?} missing from:\n{text}");
    }
    assert!(!text.contains("enabled = false"), "{text}");
    assert!(!text.contains("limit"), "{text}");
    assert_eq!(
        AgentConfig::new(directory.path()).settings().unwrap(),
        settings
    );

    let mut upgraded = git_record("https://example.com/m.git");
    upgraded.ref_name = None;
    config.set_marketplace("my-mkt", Some(&upgraded)).unwrap();
    assert!(!fs::read_to_string(&path).unwrap().contains("ref ="));

    config.set_package_settings(&id, None).unwrap();
    let settings = config.set_marketplace("my-mkt", None).unwrap();

    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("plugins"), "{text}");
    assert!(!text.contains("marketplaces"), "{text}");
    assert!(text.contains("# mine"));
    assert!(settings.plugins.packages.is_empty());
    assert!(settings.marketplaces.is_empty());
    assert!(config.set_marketplace("bad.name", None).is_err());
}

#[test]
fn config_get_and_set_name_package_and_marketplace_keys() {
    let directory = tempfile::tempdir().unwrap();
    let config = AgentConfig::new(directory.path());
    config
        .set_marketplace("my-mkt", Some(&git_record("https://example.com/m.git")))
        .unwrap();

    config
        .set_user_value(r#"plugins."demo@my-mkt".enabled"#, "false")
        .unwrap();
    config
        .set_user_value("plugins._default.enabled", "true")
        .unwrap();
    config
        .set_user_value(
            r#"plugins."demo@my-mkt".mcpServers.s.disabledTools"#,
            r#"["rm"]"#,
        )
        .unwrap();
    config
        .set_user_value("marketplaces.my-mkt.enabled", "false")
        .unwrap();

    assert_eq!(
        config.value(r#"plugins."demo@my-mkt".enabled"#).unwrap(),
        Some(json!(false))
    );
    assert_eq!(
        config.value("plugins._default.enabled").unwrap(),
        Some(json!(true))
    );
    assert_eq!(
        config.value("marketplaces.my-mkt.sourceType").unwrap(),
        Some(json!("git"))
    );
    let settings = config.settings().unwrap();
    assert_eq!(
        settings.plugins.packages["demo@my-mkt"].mcp_servers["s"].disabled_tools,
        Some(vec!["rm".to_string()])
    );
    assert_eq!(settings.marketplaces["my-mkt"].enabled, Some(false));

    for (key, value) in [
        ("plugins.bad.enabled", "true"),
        (r#"plugins."demo@my-mkt".typo"#, "true"),
        (r#"plugins."demo@my-mkt".enabled"#, "\"yes\""),
        // The Agent writes a plugin's settings, checked against its schema.
        (r#"plugins."demo@my-mkt".settings.level"#, "1"),
        (r#"plugins."demo@my-mkt".settings"#, "{ level = 1 }"),
        ("marketplaces.unknown.enabled", "false"),
        ("marketplaces.my-mkt.sourceType", "svn"),
    ] {
        assert!(
            config.set_user_value(key, value).is_err(),
            "{key} = {value}"
        );
    }
}

#[test]
fn a_repaired_entry_is_rewritten_once_and_an_unknown_one_is_kept() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_SETTINGS_FILE);
    fs::write(
        &path,
        "[plugins.\"demo@m\"]\nenabled = \"yes\"\n\n[plugins.\"not an id\"]\nenabled = true\n",
    )
    .unwrap();

    let first = AgentConfig::new(directory.path());
    assert_eq!(
        first.settings().unwrap().plugins.packages["demo@m"].enabled,
        None
    );
    assert_eq!(first.recoveries().len(), 1);

    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("\"yes\""), "{text}");
    assert!(text.contains("[plugins.\"not an id\"]"), "{text}");
    let second = AgentConfig::new(directory.path());
    second.settings().unwrap();
    assert!(second.recoveries().is_empty());
}
