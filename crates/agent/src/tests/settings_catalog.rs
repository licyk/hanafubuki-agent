//! The settings catalog: what it reads from each owner, the writes by key
//! and the collections' actions, each dispatched to its owner.

use hanafubuki_agent_config::{ConfigLayers, SettingChange, SettingWriteOp};
use hanafubuki_agent_mcp::{McpApprovalPolicy, McpTransportConfig};

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

fn collection<'a>(catalog: &'a SettingsCatalog, id: &str) -> &'a SettingsCollection {
    catalog
        .collections
        .iter()
        .find(|collection| collection.id == id)
        .unwrap_or_else(|| panic!("no collection {id}"))
}

fn with_token(_: &TestPaths, config: &AgentConfig) {
    configure_test_provider(config, "http://127.0.0.1:9", None);
    config
        .mutate_providers(1, |registry| {
            registry.providers[0].api_token = Some("sk-catalog-secret".into());
            Ok(())
        })
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_catalog_reads_every_owner_and_returns_no_secret() {
    let (service, paths, _) = service_with_config("settings-catalog", with_token);
    let catalog = service.settings_catalog().await.unwrap();

    let ids = catalog
        .sections
        .iter()
        .map(|section| section.id.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"web"), "{ids:?}");
    assert!(!ids.contains(&"tui"), "a host's configuration: {ids:?}");
    let options = catalog
        .sections
        .iter()
        .find(|section| section.id == "pluginOptions.subagents")
        .unwrap();
    assert_eq!(options.group, SettingsGroup::Plugins);
    assert_eq!(options.values.len(), options.settings.len());
    assert!(options
        .settings
        .iter()
        .all(|spec| spec.key.starts_with("pluginOptions.subagents.")));

    let collections = catalog
        .collections
        .iter()
        .map(|collection| collection.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        collections,
        [
            "providers",
            "providerModels",
            "modelPolicy",
            "mcp",
            "mcpSecrets",
            "pluginMcp",
            "plugins",
            "packages",
            "marketplaces",
            "hooks",
            "roles"
        ],
        "a host's configuration has no profiles or projects"
    );
    let providers = collection(&catalog, "providers");
    assert_eq!(providers.revision, Some(2));
    let provider = &providers.records[0];
    assert_eq!(provider.state.as_deref(), Some("default"));
    assert!(!provider.values.contains_key("apiToken"));
    assert_eq!(provider.secrets_set, ["apiToken"]);
    let policy = &collection(&catalog, "modelPolicy").records[0];
    assert_eq!(
        policy.values["model"],
        SettingValue::Text("fixture-model".into())
    );
    let roles = collection(&catalog, "roles");
    assert!(roles.records.iter().any(|role| role.id == DEFAULT_ROLE));
    let text = serde_json::to_string(&catalog).unwrap();
    assert!(
        !text.contains("sk-catalog-secret"),
        "a secret is never returned"
    );
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_by_key_takes_settings_and_plugin_options_together() {
    let (service, paths, config) = service_with_config("settings-catalog-write", |_, _| {});
    let set = |key: &str, value: serde_json::Value| SettingChange {
        key: key.to_string(),
        op: SettingWriteOp::Set(value),
    };
    let outcome = service
        .write_setting_values(SettingsWrite {
            changes: vec![
                set("webFetchUseJinaReader", json!(false)),
                set("pluginOptions.subagents.maximumRunningAgents", json!(2)),
            ],
        })
        .await
        .unwrap();
    assert_eq!(outcome.written.len(), 2);
    assert!(!config.settings().unwrap().web_fetch_use_jina_reader);
    let catalog = service.settings_catalog().await.unwrap();
    let state = catalog
        .sections
        .iter()
        .flat_map(|section| &section.values)
        .find(|state| state.key == "pluginOptions.subagents.maximumRunningAgents")
        .unwrap();
    assert_eq!(state.value, Some(SettingValue::Integer(2)));
    assert_eq!(state.source, SettingSource::User);

    let refused = service
        .write_setting_values(SettingsWrite {
            changes: vec![
                set("webFetchUseJinaReader", json!(true)),
                set("pluginOptions.subagents.maximumRunningAgents", json!(0)),
            ],
        })
        .await
        .unwrap_err();
    assert!(refused.contains("maximumRunningAgents"), "{refused}");
    assert!(
        !config.settings().unwrap().web_fetch_use_jina_reader,
        "a refused write writes nothing"
    );
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_providers_records_round_trip_and_keep_their_secret() {
    let (service, paths, config) = service_with_config("settings-catalog-providers", with_token);
    let added = service
        .run_settings_action(action(
            "providers",
            None,
            "add",
            Some(2),
            json!({
                "displayName": "Second",
                "baseUrl": "https://example.invalid/v1",
                "protocol": "chat_completions",
                "apiToken": "sk-second",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(added.record.as_deref(), Some("provider-2"));

    let stale = service
        .run_settings_action(action(
            "providers",
            Some("provider-1"),
            "save",
            Some(2),
            json!({"displayName": "Renamed"}),
        ))
        .await
        .unwrap_err();
    assert!(stale.contains("stale"), "{stale}");
    let unnamed = service
        .run_settings_action(action(
            "providers",
            Some("provider-1"),
            "save",
            None,
            json!({"displayName": "Renamed"}),
        ))
        .await
        .unwrap_err();
    assert!(unnamed.contains("revision"), "{unnamed}");
    let unknown = service
        .run_settings_action(action(
            "providers",
            Some("provider-1"),
            "save",
            Some(3),
            json!({"models": {}}),
        ))
        .await
        .unwrap_err();
    assert!(unknown.contains("not a field"), "{unknown}");

    let malformed = service
        .run_settings_action(action(
            "providers",
            Some("provider-1"),
            "save",
            Some(3),
            json!({"requestHeaders": ["X-Unknown"]}),
        ))
        .await
        .unwrap_err();
    assert!(malformed.contains("NAME=VALUE"), "{malformed}");
    service
        .run_settings_action(action(
            "providers",
            Some("provider-1"),
            "save",
            Some(3),
            json!({"displayName": "Renamed", "enabled": false, "requestHeaders": ["X-Team = blue=green"]}),
        ))
        .await
        .unwrap();
    let registry = config.providers().unwrap();
    let first = &registry.providers[0];
    assert_eq!(first.display_name, "Renamed");
    assert_eq!(
        (
            first.request_headers[0].name.as_str(),
            first.request_headers[0].value.as_str()
        ),
        ("X-Team", "blue=green")
    );
    // A header's value is never returned, and a name alone keeps it.
    let catalog = service.settings_catalog().await.unwrap();
    assert_eq!(
        collection(&catalog, "providers").records[0].values["requestHeaders"],
        SettingValue::TextList(vec!["X-Team".into()])
    );
    assert!(!serde_json::to_string(&catalog)
        .unwrap()
        .contains("blue=green"));
    service
        .run_settings_action(action(
            "providers",
            Some("provider-1"),
            "save",
            Some(config.providers().unwrap().revision),
            json!({"requestHeaders": ["X-Team", "X-Other=1"]}),
        ))
        .await
        .unwrap();
    let headers = &config.providers().unwrap().providers[0].request_headers;
    assert_eq!(
        (headers[0].value.as_str(), headers[1].value.as_str()),
        ("blue=green", "1")
    );
    assert_eq!(
        first.api_token.as_deref(),
        Some("sk-catalog-secret"),
        "a save that leaves the secret out keeps it"
    );
    assert_eq!(
        registry.default_provider_id.as_deref(),
        Some("provider-2"),
        "the disabled default gave way"
    );

    service
        .run_settings_action(action(
            "providers",
            Some("provider-2"),
            "delete",
            Some(config.providers().unwrap().revision),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(config.providers().unwrap().providers.len(), 1);
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_model_policy_is_one_record() {
    let (service, paths, config) = service_with_config("settings-catalog-policy", with_token);
    service
        .run_settings_action(action(
            "modelPolicy",
            Some("policy"),
            "save",
            Some(2),
            json!({"model": "other-model", "contextWindowTokens": 4096}),
        ))
        .await
        .unwrap();
    let policy = config.providers().unwrap().model_policy;
    assert_eq!(policy.model, "other-model");
    assert_eq!(policy.context_window_tokens, Some(4096));
    service
        .run_settings_action(action(
            "modelPolicy",
            Some("policy"),
            "save",
            Some(3),
            json!({"contextWindowTokens": null}),
        ))
        .await
        .unwrap();
    assert_eq!(
        config
            .providers()
            .unwrap()
            .model_policy
            .context_window_tokens,
        None
    );
    let refused = service
        .run_settings_action(action(
            "modelPolicy",
            Some("policy"),
            "save",
            Some(4),
            json!({"autoCompactThresholdPercent": 0}),
        ))
        .await
        .unwrap_err();
    assert!(refused.contains("autoCompactThresholdPercent"), "{refused}");

    let catalog = service.settings_catalog().await.unwrap();
    let record = &collection(&catalog, "modelPolicy").records[0];
    assert_eq!(
        record.values["customTurnBudget"],
        SettingValue::Boolean(false)
    );
    assert_eq!(
        record.values["turnBudget.maximumToolInvocations"],
        SettingValue::Integer(256),
        "the Agent's budget, to start from"
    );
    service
        .run_settings_action(action(
            "modelPolicy",
            Some("policy"),
            "save",
            Some(4),
            json!({"customTurnBudget": true, "turnBudget.maximumToolInvocations": 32}),
        ))
        .await
        .unwrap();
    let budget = config
        .providers()
        .unwrap()
        .model_policy
        .turn_budget
        .unwrap();
    assert_eq!(budget.maximum_tool_invocations, 32);
    assert_eq!(budget.maximum_in_flight_tools, 8);
    service
        .run_settings_action(action(
            "modelPolicy",
            Some("policy"),
            "save",
            Some(5),
            json!({"customTurnBudget": false}),
        ))
        .await
        .unwrap();
    assert_eq!(config.providers().unwrap().model_policy.turn_budget, None);
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_policys_effort_offers_the_models_known_levels_and_takes_a_typed_one() {
    let (service, paths, config) = service_with_config("settings-catalog-effort", with_token);
    let effort_kind = |catalog: &SettingsCatalog| {
        collection(catalog, "modelPolicy")
            .record
            .iter()
            .find(|spec| spec.key == "reasoningEffort")
            .unwrap()
            .kind
            .clone()
    };
    let offered = |values: Vec<SettingChoice>| SettingKind::ChoiceOrText {
        values,
        maximum_chars: Some(64),
        nullable: true,
    };
    let catalog = service.settings_catalog().await.unwrap();
    assert_eq!(effort_kind(&catalog), offered(Vec::new()), "no level known");

    config
        .mutate_providers(2, |registry| {
            registry.providers[0].models.insert(
                "fixture-model".into(),
                AgentModelOverrideSettings {
                    reasoning_efforts: Some(vec![
                        ModelReasoningEffort::Low,
                        ModelReasoningEffort::High,
                    ]),
                    ..AgentModelOverrideSettings::default()
                },
            );
            Ok(())
        })
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    assert_eq!(
        effort_kind(&catalog),
        offered(vec![
            SettingChoice::new("low", "low"),
            SettingChoice::new("high", "high"),
        ])
    );

    let revision = |config: &AgentConfig| config.providers().unwrap().revision;
    let save = |effort: serde_json::Value| {
        action(
            "modelPolicy",
            Some("policy"),
            "save",
            Some(revision(&config)),
            json!({ "reasoningEffort": effort }),
        )
    };
    service
        .run_settings_action(save(json!("max")))
        .await
        .unwrap();
    assert_eq!(
        config.providers().unwrap().model_policy.reasoning_effort,
        Some(ModelReasoningEffort::parse("max").unwrap())
    );
    let catalog = service.settings_catalog().await.unwrap();
    assert_eq!(
        collection(&catalog, "modelPolicy").records[0].values["reasoningEffort"],
        SettingValue::Text("max".into())
    );
    let refused = service
        .run_settings_action(save(json!("none")))
        .await
        .unwrap_err();
    assert!(refused.contains("reasoning effort 'none'"), "{refused}");
    service
        .run_settings_action(save(json!(null)))
        .await
        .unwrap();
    assert_eq!(
        config.providers().unwrap().model_policy.reasoning_effort,
        None
    );
    done(service, paths);
}

#[test]
fn the_effort_choices_mark_the_models_default_and_are_none_without_levels() {
    let known = KnownReasoning::Levels {
        efforts: vec![ModelReasoningEffort::Low, ModelReasoningEffort::Medium],
        default: Some(ModelReasoningEffort::Medium),
    };
    assert_eq!(
        crate::reasoning_effort_choices(&known),
        [
            SettingChoice::new("low", "low"),
            SettingChoice::new("medium", "medium (model default)"),
        ]
    );
    for known in [
        KnownReasoning::Unknown,
        KnownReasoning::Unsupported { learned: true },
    ] {
        assert!(crate::reasoning_effort_choices(&known).is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_mcp_server_is_added_saved_and_deleted() {
    let (service, paths, config) = service_with_config("settings-catalog-mcp", |_, _| {});
    let revision = |config: &AgentConfig| config.mcp().load().unwrap().revision;
    service
        .run_settings_action(action(
            "mcp",
            Some("demo"),
            "add",
            Some(revision(&config)),
            json!({
                "enabled": false,
                "transport.program": "demo-server",
                "transport.argv": ["serve"],
                "transport.publicEnvironment": ["MODE=fast"],
                "toolApprovalPolicies": ["search=ask"],
                "toolMaximumConcurrentCalls": ["search=2"],
            }),
        ))
        .await
        .unwrap();
    let server = config.mcp().load().unwrap().servers.remove(0);
    let McpTransportConfig::Stdio(stdio) = &server.transport else {
        panic!("a stdio server");
    };
    assert_eq!(stdio.public_environment["MODE"], "fast");
    assert_eq!(
        server.tool_approval_policies["search"],
        McpApprovalPolicy::Ask
    );
    assert_eq!(server.tool_maximum_concurrent_calls["search"], 2);
    for (values, refusal) in [
        (
            json!({"toolMaximumConcurrentCalls": ["search=many"]}),
            "whole number",
        ),
        (json!({"toolApprovalPolicies": ["search"]}), "NAME=VALUE"),
        (
            json!({"transport.publicEnvironment": ["A=1", "A=2"]}),
            "twice",
        ),
    ] {
        let refused = service
            .run_settings_action(action(
                "mcp",
                Some("demo"),
                "save",
                Some(revision(&config)),
                values,
            ))
            .await
            .unwrap_err();
        assert!(refused.contains(refusal), "{refused}");
    }

    service
        .run_settings_action(action(
            "mcpSecrets",
            Some("demo/API_KEY"),
            "add",
            Some(revision(&config)),
            json!({"value": "sk-mcp-secret"}),
        ))
        .await
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let secret = &collection(&catalog, "mcpSecrets").records[0];
    assert_eq!(secret.id, "demo/API_KEY");
    assert_eq!(secret.secrets_set, ["value"]);
    assert!(!serde_json::to_string(&catalog)
        .unwrap()
        .contains("sk-mcp-secret"));
    let settings = config.mcp().load().unwrap();
    let McpTransportConfig::Stdio(stdio) = &settings.servers[0].transport else {
        panic!("a stdio server");
    };
    let key = stdio.credential_environment["API_KEY"].clone();
    assert_eq!(settings.credentials[&key].value, "sk-mcp-secret");
    service
        .run_settings_action(action(
            "mcpSecrets",
            Some("demo/API_KEY"),
            "save",
            Some(revision(&config)),
            json!({"value": "sk-mcp-other"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        config.mcp().load().unwrap().credentials[&key].value,
        "sk-mcp-other"
    );
    for (name, verb) in [("demo/OTHER", "add"), ("demo/OTHER", "delete")] {
        service
            .run_settings_action(action(
                "mcpSecrets",
                Some(name),
                verb,
                Some(revision(&config)),
                json!({"value": "sk-other"}),
            ))
            .await
            .unwrap();
    }
    let settings = config.mcp().load().unwrap();
    assert_eq!(
        settings.credentials.len(),
        1,
        "a deleted secret's credential goes"
    );
    let public = service
        .run_settings_action(action(
            "mcpSecrets",
            Some("demo/MODE"),
            "add",
            Some(revision(&config)),
            json!({"value": "x"}),
        ))
        .await
        .unwrap_err();
    assert!(public.contains("public"), "{public}");

    service
        .run_settings_action(action(
            "mcp",
            Some("demo"),
            "save",
            Some(revision(&config)),
            json!({"transport.type": "streamable_http", "transport.url": "https://example.invalid/mcp"}),
        ))
        .await
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let record = &collection(&catalog, "mcp").records[0];
    assert_eq!(record.id, "demo");
    assert_eq!(record.summary, "https://example.invalid/mcp");
    assert_eq!(
        record.values["transport.type"],
        SettingValue::Text("streamable_http".into())
    );
    assert_eq!(record.values["enabled"], SettingValue::Boolean(false));
    assert!(
        config.mcp().load().unwrap().credentials.is_empty(),
        "the stdio transport's secret went with it"
    );
    service
        .run_settings_action(action(
            "mcp",
            Some("demo"),
            "delete",
            Some(revision(&config)),
            json!({}),
        ))
        .await
        .unwrap();
    assert!(config.mcp().load().unwrap().servers.is_empty());
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_is_turned_off_through_its_record() {
    let (service, paths, config) = service_with_config("settings-catalog-plugins", |_, _| {});
    service
        .run_settings_action(action(
            "plugins",
            Some("recap"),
            "save",
            None,
            json!({"enabled": false}),
        ))
        .await
        .unwrap();
    assert!(config
        .settings()
        .unwrap()
        .disabled_plugins
        .contains("recap"));
    let sources = config.setting_sources().unwrap();
    assert!(
        !sources.contains_key("permissionMode"),
        "only disabledPlugins is written"
    );
    let unknown = service
        .run_settings_action(action("plugins", Some("recap"), "explode", None, json!({})))
        .await
        .unwrap_err();
    assert!(unknown.contains("no action `explode`"), "{unknown}");
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_standalone_configuration_has_its_profiles_and_projects() {
    let paths = test_paths("settings-catalog-layered");
    let config = AgentConfig::layered(&paths.config_dir, ConfigLayers::default());
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
    let project = paths.data_dir.join("some project");
    let project = project.display().to_string();
    service
        .run_settings_action(action("projects", Some(&project), "add", None, json!({})))
        .await
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    assert!(catalog.sections.iter().any(|section| section.id == "tui"));
    let profile = &collection(&catalog, "profiles").records[0];
    assert_eq!(profile.id, "ci");
    assert_eq!(
        profile.values["permissionMode"],
        SettingValue::Text("full_access".into())
    );
    assert_eq!(
        profile.values["memory.enabled"],
        SettingValue::Boolean(true)
    );
    let trusted = &collection(&catalog, "projects").records[0];
    assert_eq!(trusted.id, project);
    assert_eq!(trusted.state.as_deref(), Some("trusted"));
    service
        .run_settings_action(action(
            "projects",
            Some(&project),
            "untrust",
            None,
            json!({}),
        ))
        .await
        .unwrap();
    let value = config
        .value(&format!("projects.{}.trustLevel", json!(project)))
        .unwrap();
    assert_eq!(value, Some(json!("untrusted")));
    let refused = service
        .run_settings_action(action(
            "profiles",
            Some("ci"),
            "save",
            None,
            json!({"memory.maxSourceAgeDays": 0}),
        ))
        .await
        .unwrap_err();
    assert!(refused.contains("maxSourceAgeDays"), "{refused}");
    done(service, paths);
}

#[test]
fn an_action_request_is_strict_and_hides_its_values() {
    let refused = serde_json::from_value::<SettingsActionRequest>(json!({
        "collection": "providers",
        "action": "save",
        "surprise": true,
    }));
    assert!(refused.is_err());
    let request = action(
        "providers",
        Some("provider-1"),
        "save",
        Some(1),
        json!({"apiToken": "sk-hidden"}),
    );
    let debug = format!("{request:?}");
    assert!(debug.contains("apiToken"), "{debug}");
    assert!(!debug.contains("sk-hidden"), "{debug}");
}
