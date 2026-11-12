use std::path::PathBuf;

use hanafubuki_agent_config::{
    AgentConfig, ConfigLayers, SettingApply, SettingSource, SettingsWrite, AGENT_SETTINGS_FILE,
};
use serde_json::json;

use super::{reset, set, setup, write};

const USER: &str = r#"# The user's own comment.
permissionMode = "auto_review"
someFutureKey = "kept"

[memory]
enabled = true
maxUnusedDays = 40

[profiles.work]
logLevel = "debug"
"#;

#[test]
fn a_set_and_a_reset_change_only_their_keys() {
    let fixture = setup(USER, "");
    let config = fixture.layered(None, &[]);
    let outcome = config
        .write_values(&write(vec![
            set("memory.maxSourceAgeDays", json!(20)),
            set("tui.theme", json!("dracula")),
            set("logLevel", json!("warn")),
        ]))
        .unwrap();
    assert_eq!(
        outcome.written,
        ["memory.maxSourceAgeDays", "tui.theme", "logLevel"]
    );
    assert!(outcome.shadowed.is_empty());
    assert_eq!(
        outcome.apply,
        [
            ("memory.maxSourceAgeDays".to_string(), SettingApply::Live),
            ("tui.theme".to_string(), SettingApply::Live),
            ("logLevel".to_string(), SettingApply::NextLaunch),
        ]
    );
    let file = fixture.user_file();
    assert!(
        file.starts_with("# The user's own comment.\npermissionMode = \"auto_review\"\n"),
        "{file}"
    );
    assert!(file.contains("someFutureKey = \"kept\""), "{file}");
    assert!(file.contains("maxSourceAgeDays = 20"), "{file}");
    assert!(file.contains("theme = \"dracula\""), "{file}");
    assert_eq!(config.settings().unwrap().memory.max_source_age_days, 20);
    assert_eq!(
        config.tui_preferences().unwrap().theme.as_deref(),
        Some("dracula")
    );

    config
        .write_values(&write(vec![
            reset("memory.maxUnusedDays"),
            reset("tui.theme"),
            // Resetting a key the file does not set is no change.
            reset("memory.minimumIdleHours"),
        ]))
        .unwrap();
    let file = fixture.user_file();
    assert!(!file.contains("maxUnusedDays"), "{file}");
    assert!(!file.contains("dracula"), "{file}");
    assert!(file.contains("someFutureKey = \"kept\""), "{file}");
    let settings = config.settings().unwrap();
    assert_eq!(settings.memory.max_unused_days, 30, "the default again");
    assert!(settings.memory.enabled);
    assert_eq!(config.tui_preferences().unwrap().theme, None);
}

#[test]
fn a_refused_write_changes_nothing() {
    let fixture = setup(USER, "");
    let config = fixture.layered(None, &[]);
    let before = fixture.user_file();
    for (changes, expected) in [
        (
            vec![set("memory.maxUnusedDays", json!(0))],
            "`memory.maxUnusedDays` cannot take this value: it is between 1 and 3650",
        ),
        (
            vec![set("permissionMode", json!("everything"))],
            "is not one of manual, auto_review, full_access",
        ),
        (
            vec![set("memory.enabled", json!("yes"))],
            "is not true or false",
        ),
        (vec![set("memory.unknown", json!(1))], "not a setting"),
        (vec![reset("plugins")], "not a setting"),
        // A value its spec takes that the settings still cannot use.
        (
            vec![set("network.proxy", json!("ftp://proxy"))],
            "cannot take this value",
        ),
        (
            vec![set("memory.extractionModel", json!("   "))],
            "cannot take this value",
        ),
        // A good value does not carry a bad one with it.
        (
            vec![
                set("memory.enabled", json!(false)),
                set("compaction.background.thresholdPercent", json!(101)),
            ],
            "between 1 and 100",
        ),
    ] {
        let error = config.write_values(&write(changes)).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    assert_eq!(fixture.user_file(), before);
}

#[test]
fn a_commit_group_is_checked_as_one_write() {
    let fixture = setup("", "");
    let config = fixture.layered(None, &[]);
    let error = config
        .write_values(&write(vec![set("webSearchProvider", json!("searxng"))]))
        .unwrap_err();
    assert!(error.contains("SearXNG has no usable endpoint"), "{error}");
    assert_eq!(fixture.user_file(), "");

    config
        .write_values(&write(vec![
            set("webSearchProvider", json!("searxng")),
            set("searxngBaseUrl", json!("https://search.example.test/")),
        ]))
        .unwrap();
    let settings = config.settings().unwrap();
    assert_eq!(
        settings.searxng_base_url.as_deref(),
        Some("https://search.example.test")
    );

    // Removing the endpoint alone would leave SearXNG without one.
    let error = config
        .write_values(&write(vec![set("searxngBaseUrl", json!(null))]))
        .unwrap_err();
    assert!(error.contains("cannot take this value"), "{error}");
    config
        .write_values(&write(vec![
            reset("webSearchProvider"),
            set("searxngBaseUrl", json!(null)),
        ]))
        .unwrap();
    assert!(!fixture.user_file().contains("searxng"));
}

#[test]
fn each_setting_names_the_layer_its_value_comes_from() {
    let fixture = setup(
        "[profiles.work]\nwebFetchUseJinaReader = true\n[memory]\nenabled = true\n",
        "[memory]\nmaxUnusedDays = 12\n",
    );
    fixture.trust_project(&fixture.layered(None, &[]));
    let config = fixture.layered(
        Some("work"),
        // An override the settings cannot use does not take the key.
        &["logLevel=trace", "memory.enabled=3"],
    );
    let sources = config.setting_sources().unwrap();
    assert_eq!(sources["memory.enabled"], SettingSource::User);
    assert_eq!(
        sources["webFetchUseJinaReader"],
        SettingSource::Profile {
            name: "work".into()
        }
    );
    assert_eq!(
        sources["memory.maxUnusedDays"],
        SettingSource::Project {
            path: ConfigLayers::project_file(&fixture.project)
        }
    );
    assert_eq!(sources["logLevel"], SettingSource::Override);
    assert!(!sources.contains_key("permissionMode"), "the default's");

    let outcome = config
        .write_values(&write(vec![
            set("logLevel", json!("warn")),
            set("webFetchUseJinaReader", json!(false)),
            set("permissionMode", json!("full_access")),
        ]))
        .unwrap();
    assert_eq!(
        outcome.shadowed,
        [
            ("logLevel".to_string(), SettingSource::Override),
            (
                "webFetchUseJinaReader".to_string(),
                SettingSource::Profile {
                    name: "work".into()
                }
            ),
        ]
    );
    assert_eq!(
        config.setting_sources().unwrap()["permissionMode"],
        SettingSource::User
    );
}

#[test]
fn a_hosts_configuration_has_the_user_layer_alone() {
    let fixture = setup(USER, "");
    let config = AgentConfig::new(&fixture.config_dir);
    let sources = config.setting_sources().unwrap();
    assert_eq!(sources["memory.maxUnusedDays"], SettingSource::User);
    assert!(!sources.contains_key("logLevel"), "a host supplies its own");
    let outcome = config
        .write_values(&write(vec![set("memory.maxUnusedDays", json!(7))]))
        .unwrap();
    assert!(outcome.shadowed.is_empty());
    assert_eq!(config.settings().unwrap().memory.max_unused_days, 7);
    let path: PathBuf = fixture.config_dir.join(AGENT_SETTINGS_FILE);
    assert!(std::fs::read_to_string(path)
        .unwrap()
        .contains("maxUnusedDays = 7"));
}

#[test]
fn config_set_writes_as_write_values_does() {
    let by_set = setup(USER, "");
    let by_write = setup(USER, "");
    by_set
        .layered(None, &[])
        .set_user_value("compaction.hierarchical.protectedTools", "[\"shell\"]")
        .unwrap();
    by_write
        .layered(None, &[])
        .write_values(&write(vec![set(
            "compaction.hierarchical.protectedTools",
            json!(["shell"]),
        )]))
        .unwrap();
    assert_eq!(by_set.user_file(), by_write.user_file());
}

#[test]
fn a_write_from_a_frontend_is_strict() {
    let parsed: SettingsWrite = serde_json::from_value(json!({
        "changes": [
            {"key": "memory.enabled", "op": {"set": true}},
            {"key": "tui.theme", "op": "reset"}
        ]
    }))
    .unwrap();
    assert_eq!(
        parsed,
        write(vec![set("memory.enabled", json!(true)), reset("tui.theme")])
    );
    for refused in [
        json!({"changes": [], "dryRun": true}),
        json!({"changes": [{"key": "memory.enabled", "op": "reset", "layer": "user"}]}),
        json!({"changes": [{"key": "memory.enabled", "op": "unset"}]}),
    ] {
        assert!(serde_json::from_value::<SettingsWrite>(refused).is_err());
    }
}
