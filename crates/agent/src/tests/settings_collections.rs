//! The collections of the settings catalog that write what only the files
//! held before: a record's add starting from its defaults, a profile's key
//! and a profile removed, the user's roles, a hook's definition shown, a
//! model's overrides, the installed plugins, their MCP servers and their
//! code's settings.

use hanafubuki_agent_config::{ConfigLayers, SettingChange, SettingWriteOp};

use super::packages::{claude_plugin, install, set_enabled, write};
use super::*;

fn done(service: AgentApplicationService, paths: TestPaths) {
    let _ = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

fn action(
    collection: &str,
    record: Option<&str>,
    action: &str,
    revision: Option<u64>,
    values: serde_json::Value,
) -> SettingsActionRequest {
    serde_json::from_value(json!({
        "collection": collection,
        "record": record,
        "action": action,
        "expectedRevision": revision,
        "values": values,
    }))
    .unwrap()
}

fn collection(catalog: &SettingsCatalog, id: &str) -> SettingsCollection {
    catalog
        .collections
        .iter()
        .find(|collection| collection.id == id)
        .cloned()
        .unwrap_or_else(|| panic!("no collection {id}"))
}

fn default_of(collection: &SettingsCollection, key: &str) -> Option<SettingValue> {
    collection
        .record
        .iter()
        .find(|spec| spec.key == key)
        .unwrap_or_else(|| panic!("no field {key}"))
        .default
        .clone()
}

/// A standalone configuration whose user file has the profiles `profiles`,
/// with `profile` selected.
fn layered(
    name: &str,
    profiles: &str,
    profile: Option<&str>,
) -> (AgentApplicationService, TestPaths, AgentConfig) {
    let paths = test_paths(name);
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::write(paths.config_dir.join("config.toml"), profiles).unwrap();
    let config = AgentConfig::layered(
        &paths.config_dir,
        ConfigLayers {
            profile: profile.map(str::to_string),
            ..ConfigLayers::default()
        },
    );
    let service = AgentApplicationService::new_with_host(
        config.clone(),
        test_host(&paths),
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        hanafubuki_agent_process::executor::ExecutorManager::default(),
    )
    .unwrap();
    (service, paths, config)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_add_starts_from_the_values_it_writes() {
    let (service, paths, config) = service_with_config("settings-defaults", |_, _| {});
    let catalog = service.settings_catalog().await.unwrap();
    let providers = collection(&catalog, "providers");
    assert_eq!(
        default_of(&providers, "enabled"),
        Some(SettingValue::Boolean(true))
    );
    assert_eq!(
        default_of(&providers, "protocol"),
        Some(SettingValue::Text("responses".into()))
    );
    let mcp = collection(&catalog, "mcp");
    assert_eq!(
        default_of(&mcp, "transport.type"),
        Some(SettingValue::Text("stdio".into()))
    );

    let revision = config.mcp().load().unwrap().revision;
    service
        .run_settings_action(action(
            "mcp",
            Some("fresh"),
            "add",
            Some(revision),
            json!({"transport.program": "fresh-server"}),
        ))
        .await
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let mcp = collection(&catalog, "mcp");
    let record = &mcp.records[0];
    for spec in &mcp.record {
        let shown = spec
            .visible_if
            .as_ref()
            .is_none_or(|condition| record.values.get(&condition.key) == Some(&condition.equals));
        if spec.key == "transport.program" || !shown {
            continue;
        }
        match &spec.default {
            Some(default) => assert_eq!(
                record.values.get(&spec.key),
                Some(default),
                "{} is what the add form showed",
                spec.key
            ),
            None => assert!(
                !record.values.contains_key(&spec.key)
                    || record.values[&spec.key] == SettingValue::TextList(Vec::new()),
                "{} has no default and was not written",
                spec.key
            ),
        }
    }
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_profiles_key_and_a_profile_are_removed_but_not_the_selected_one() {
    let (service, paths, config) = layered(
        "settings-profiles-remove",
        "[profiles.work]\nmemory.enabled = false\n",
        Some("work"),
    );
    service
        .run_settings_action(action(
            "profiles",
            Some("ci"),
            "add",
            None,
            json!({"permissionMode": "full_access", "memory.enabled": true}),
        ))
        .await
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let profiles = collection(&catalog, "profiles");
    assert!(
        profiles.record.iter().all(|spec| spec.default.is_none()),
        "a new profile sets nothing"
    );
    service
        .run_settings_action(action(
            "profiles",
            Some("ci"),
            "save",
            None,
            json!({"memory.enabled": null}),
        ))
        .await
        .unwrap();
    assert_eq!(config.value("profiles.ci.memory.enabled").unwrap(), None);
    assert_eq!(
        config.value("profiles.ci.permissionMode").unwrap(),
        Some(json!("full_access"))
    );
    let selected = service
        .run_settings_action(action("profiles", Some("work"), "delete", None, json!({})))
        .await
        .unwrap_err();
    assert!(selected.contains("runs with"), "{selected}");
    service
        .run_settings_action(action("profiles", Some("ci"), "delete", None, json!({})))
        .await
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let names = collection(&catalog, "profiles")
        .records
        .iter()
        .map(|record| record.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(names, ["work"]);
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_users_roles_are_written_and_the_others_are_shown() {
    let (service, paths, _config) = service_with_config("settings-roles", |_, _| {});
    let empty = service
        .run_settings_action(action("roles", Some("reviewer"), "add", None, json!({})))
        .await
        .unwrap_err();
    assert!(empty.contains("at least one"), "{empty}");
    service
        .run_settings_action(action(
            "roles",
            Some("reviewer"),
            "add",
            None,
            json!({"description": "Reviews a change.", "reasoningEffort": "high"}),
        ))
        .await
        .unwrap();
    service
        .run_settings_action(action(
            "roles",
            Some("reviewer"),
            "save",
            None,
            json!({"spawnable": false, "reasoningEffort": null}),
        ))
        .await
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let roles = collection(&catalog, "roles");
    let reviewer = roles
        .records
        .iter()
        .find(|role| role.id == "reviewer")
        .unwrap();
    assert_eq!(reviewer.read_only, None);
    assert_eq!(
        reviewer.values["description"],
        SettingValue::Text("Reviews a change.".into())
    );
    assert_eq!(reviewer.values["spawnable"], SettingValue::Boolean(false));
    assert!(!reviewer.values.contains_key("reasoningEffort"));
    let role = service.roles();
    let role = role.role("reviewer").unwrap();
    assert!(!role.spawnable && role.reasoning_effort.is_none());
    let file = std::fs::read_to_string(paths.config_dir.join("config.toml")).unwrap();
    assert!(file.contains("[agents.reviewer]"), "{file}");

    let builtin = roles
        .records
        .iter()
        .find(|role| role.id == DEFAULT_ROLE)
        .unwrap();
    let reason = builtin.read_only.clone().expect("a built-in role is shown");
    let refused = service
        .run_settings_action(action(
            "roles",
            Some(DEFAULT_ROLE),
            "save",
            None,
            json!({"spawnable": false}),
        ))
        .await
        .unwrap_err();
    assert_eq!(refused, reason);
    let effort = service
        .run_settings_action(action(
            "roles",
            Some("reviewer"),
            "save",
            None,
            json!({"reasoningEffort": "none"}),
        ))
        .await
        .unwrap_err();
    assert!(effort.contains("none"), "{effort}");
    service
        .run_settings_action(action("roles", Some("reviewer"), "delete", None, json!({})))
        .await
        .unwrap();
    assert!(service.roles().role("reviewer").is_none());
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_models_overrides_are_added_saved_and_deleted_at_the_revision() {
    let (service, paths, config) = service_with_config("settings-model-overrides", |_, config| {
        configure_test_provider(config, "http://127.0.0.1:9", None);
    });
    let revision = |config: &AgentConfig| config.providers().unwrap().revision;
    let provider = config.providers().unwrap().providers[0].id.clone();
    let record = format!("{provider}/vendor/model-1");
    let catalog = service.settings_catalog().await.unwrap();
    assert_eq!(
        default_of(&collection(&catalog, "providerModels"), "imageInput"),
        Some(SettingValue::Text("auto".into()))
    );
    service
        .run_settings_action(action(
            "providerModels",
            Some(&record),
            "add",
            Some(revision(&config)),
            json!({"contextWindowTokens": 64000, "reasoningEfforts": ["low", "high"]}),
        ))
        .await
        .unwrap();
    let models = &config.providers().unwrap().providers[0].models;
    let overrides = &models["vendor/model-1"];
    assert_eq!(overrides.context_window_tokens, Some(64000));
    assert_eq!(overrides.reasoning_efforts.as_ref().unwrap().len(), 2);

    let stale = service
        .run_settings_action(action(
            "providerModels",
            Some(&record),
            "save",
            Some(revision(&config) - 1),
            json!({"maxOutputTokens": 1000}),
        ))
        .await
        .unwrap_err();
    assert!(!stale.is_empty());
    let zero = service
        .run_settings_action(action(
            "providerModels",
            Some(&record),
            "save",
            Some(revision(&config)),
            json!({"contextWindowTokens": 0}),
        ))
        .await
        .unwrap_err();
    assert!(zero.contains("contextWindowTokens"), "{zero}");
    service
        .run_settings_action(action(
            "providerModels",
            Some(&record),
            "save",
            Some(revision(&config)),
            json!({"contextWindowTokens": null, "imageInput": "unsupported"}),
        ))
        .await
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let saved = &collection(&catalog, "providerModels").records[0];
    assert_eq!(saved.id, record);
    assert!(!saved.values.contains_key("contextWindowTokens"));
    assert_eq!(
        saved.values["imageInput"],
        SettingValue::Text("unsupported".into())
    );
    service
        .run_settings_action(action(
            "providerModels",
            Some(&record),
            "delete",
            Some(revision(&config)),
            json!({}),
        ))
        .await
        .unwrap();
    assert!(config.providers().unwrap().providers[0].models.is_empty());
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hooks_record_shows_its_whole_definition() {
    let (service, paths) = service_with("settings-hooks", |paths, _| {
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(
            paths.config_dir.join("hooks.json"),
            serde_json::to_vec(&json!({ "hooks": { "PreToolUse": [{
                "matcher": "shell",
                "hooks": [{ "type": "command", "command": "check-it", "timeout": 7, "statusMessage": "Checking" }],
            }] } }))
            .unwrap(),
        )
        .unwrap();
    });
    let catalog = service.settings_catalog().await.unwrap();
    let hooks = collection(&catalog, "hooks");
    let [hook] = hooks.records.as_slice() else {
        panic!("one hook: {:?}", hooks.records);
    };
    assert!(hook.label.ends_with("· shell"), "{}", hook.label);
    for part in ["check-it", "7s", "\"Checking\"", "hooks.json"] {
        assert!(hook.summary.contains(part), "{part}: {}", hook.summary);
    }
    assert_eq!(hook.state.as_deref(), Some("untrusted"));
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_installed_plugin_is_installed_turned_off_and_uninstalled_through_its_records() {
    let (service, paths, config) = service_with_config("settings-packages", |_, _| {});
    let source = claude_plugin(&paths, "demo", json!({}), None);
    let installed = service
        .run_settings_action(action(
            "packages",
            Some(&source.display().to_string()),
            "add",
            None,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(installed.record.as_deref(), Some("demo@local"));
    assert!(installed.message.unwrap().contains("version 1.0.0"));
    service
        .run_settings_action(action(
            "packages",
            Some("demo@local"),
            "save",
            None,
            json!({"enabled": false}),
        ))
        .await
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let record = &collection(&catalog, "packages").records[0];
    assert_eq!(record.id, "demo@local");
    assert_eq!(record.state.as_deref(), Some("off"));
    assert_eq!(record.values["enabled"], SettingValue::Boolean(false));
    assert_eq!(
        config.settings().unwrap().plugins.packages["demo@local"].enabled,
        Some(false)
    );
    service
        .run_settings_action(action(
            "packages",
            Some("demo@local"),
            "delete",
            None,
            json!({}),
        ))
        .await
        .unwrap();
    assert!(service.packages().await.unwrap().is_empty());
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_code_plugins_settings_are_a_section_written_on_their_own() {
    let (service, paths, config) = service_with_config("settings-package-options", |_, _| {});
    let source = paths.app_data_dir.join("sources/tuned");
    write(
        &source.join(".hanafubuki-plugin/plugin.json"),
        &json!({
            "name": "tuned",
            "version": "1.0.0",
            "runtime": { "type": "process", "command": ["./bin/guest"] },
            "permissions": { "workspace": "none" },
            "configSchema": {
                "type": "object",
                "properties": {
                    "level": { "type": "integer", "title": "Level", "minimum": 1, "maximum": 5, "default": 2 },
                    "label": { "type": "string" },
                },
            },
        })
        .to_string(),
    );
    write(&source.join("bin/guest"), "not a program");
    install(&paths, &source, "tuned@local");
    set_enabled(&config, "tuned@local", true);
    service.reload_packages().await.unwrap();

    let catalog = service.settings_catalog().await.unwrap();
    let section = catalog
        .sections
        .iter()
        .find(|section| section.id == "plugins.\"tuned@local\".settings")
        .expect("the plugin's settings are a section");
    assert_eq!(section.group, SettingsGroup::Plugins);
    let level = section
        .settings
        .iter()
        .find(|spec| spec.key == "plugins.\"tuned@local\".settings.level")
        .unwrap();
    assert_eq!(level.label, "Level");

    let set = |key: &str, value: serde_json::Value| SettingChange {
        key: key.to_string(),
        op: SettingWriteOp::Set(value),
    };
    let level_key = "plugins.\"tuned@local\".settings.level";
    service
        .write_setting_values(SettingsWrite {
            changes: vec![set(level_key, json!(4))],
        })
        .await
        .unwrap();
    let id = hanafubuki_agent_packages::PluginId::parse("tuned@local").unwrap();
    let settings = |config: &AgentConfig| {
        config.settings().unwrap().plugins.packages[&id.key()]
            .settings
            .clone()
    };
    assert_eq!(
        settings(&config).map(serde_json::Value::Object),
        Some(json!({ "level": 4 }))
    );
    let refused = service
        .write_setting_values(SettingsWrite {
            changes: vec![set(level_key, json!(9))],
        })
        .await
        .unwrap_err();
    assert!(refused.contains("between 1 and 5"), "{refused}");
    let mixed = service
        .write_setting_values(SettingsWrite {
            changes: vec![
                set(level_key, json!(3)),
                set("webFetchUseJinaReader", json!(false)),
            ],
        })
        .await
        .unwrap_err();
    assert!(mixed.contains("on their own"), "{mixed}");
    service
        .write_setting_values(SettingsWrite {
            changes: vec![SettingChange {
                key: level_key.to_string(),
                op: SettingWriteOp::Reset,
            }],
        })
        .await
        .unwrap();
    assert_eq!(settings(&config), None);
    done(service, paths);
}
