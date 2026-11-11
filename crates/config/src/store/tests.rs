use std::fs;

use hanafubuki_agent_mcp::{
    McpCredentialKey, McpCredentialSettings, McpError, McpServerConfig, McpSettingsRepairKind,
};
use serde_json::json;

use super::*;
use std::collections::BTreeMap;

use crate::{
    AgentConfigProblem, AgentLogLevel, AgentModelDiscovery, AgentModelImageInput,
    AgentModelOverrideSettings, AgentNetworkSettings, AgentPermissionMode, AgentProviderSettings,
    AgentProxy, AgentSandboxMode, AgentSandboxWorkspaceWrite, AgentWebSearchProvider,
};

fn recovered(directory: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(directory.join(".recovery")) else {
        return Vec::new();
    };
    entries
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect()
}

fn provider(id: &str) -> AgentProviderSettings {
    AgentProviderSettings {
        id: id.into(),
        display_name: "Example".into(),
        enabled: true,
        protocol: crate::AgentModelProtocol::Responses,
        base_url: "https://api.example.test/v1".into(),
        api_token: Some("sk-verbatim token".into()),
        request_headers: Vec::new(),
        models: Default::default(),
    }
}

fn stdio_server() -> McpServerConfig {
    serde_json::from_value(json!({
        "id": "local",
        "enabled": true,
        "required": false,
        "transport": {
            "type": "stdio",
            "program": "fake-mcp",
            "argv": ["--stdio"],
            "cwd": null,
            "publicEnvironment": {"LANG": "C"},
            "credentialEnvironment": {"TOKEN": "mcp-local-token"}
        },
        "connectTimeoutSeconds": 10,
        "initializeTimeoutSeconds": 10,
        "listTimeoutSeconds": 10,
        "callTimeoutSeconds": 30,
        "shutdownTimeoutSeconds": 10,
        "maximumConcurrentCalls": 1,
        "toolMaximumConcurrentCalls": {},
        "enabledTools": [],
        "disabledTools": [],
        "approvalPolicy": "ask",
        "toolApprovalPolicies": {},
        "exposurePolicy": "direct",
        "toolExposurePolicies": {}
    }))
    .unwrap()
}

#[test]
fn a_missing_configuration_reads_as_the_defaults_and_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let config = AgentConfig::new(directory.path());

    assert_eq!(config.settings().unwrap(), AgentSettings::default());
    assert_eq!(
        config.providers().unwrap(),
        AgentProviderRegistrySettings::default()
    );
    assert_eq!(config.mcp().load().unwrap(), McpSettings::default());
    assert!(!AgentConfig::exists(directory.path()));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn a_standalone_patch_writes_only_its_keys_and_keeps_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_SETTINGS_FILE);
    fs::write(&path, "# proxy\nfutureKey = 3\n").unwrap();
    let config = AgentConfig::layered(directory.path(), ConfigLayers::default());

    let written = config
        .write_standalone_settings(AgentStandaloneSettingsPatch {
            network: Some(AgentNetworkSettings {
                proxy: AgentProxy::Disabled,
            }),
            log_level: Some(AgentLogLevel::Debug),
        })
        .unwrap();

    assert_eq!(written.network.proxy, AgentProxy::Disabled);
    assert_eq!(written.log_level, AgentLogLevel::Debug);
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("# proxy\n"), "{text}");
    assert!(text.contains("futureKey = 3"), "{text}");
    assert!(text.contains("proxy = \"disabled\""), "{text}");
    assert!(text.contains("logLevel = \"debug\""), "{text}");
}
#[test]
fn a_write_keeps_comments_and_unknown_keys_and_reads_back() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_SETTINGS_FILE);
    fs::write(
        &path,
        "# chosen by hand\npermissionMode = \"auto_review\" # trust reviews\nfutureKey = 3\n\n[memory]\n# idle first\nminimumIdleHours = 12\n",
    )
    .unwrap();
    let config = AgentConfig::new(directory.path());
    assert_eq!(
        config.settings().unwrap().permission_mode,
        AgentPermissionMode::AutoReview
    );
    assert_eq!(config.settings().unwrap().memory.minimum_idle_hours, 12);

    let written = config
        .write_settings(AgentSettingsPatch {
            web_search_provider: Some(AgentWebSearchProvider::Searxng),
            searxng_base_url: Some(Some("https://search.example.test/".into())),
            ..AgentSettingsPatch::default()
        })
        .unwrap();

    assert_eq!(
        written.searxng_base_url.as_deref(),
        Some("https://search.example.test")
    );
    let text = fs::read_to_string(&path).unwrap();
    for kept in [
        "# chosen by hand",
        "permissionMode = \"auto_review\" # trust reviews",
        "futureKey = 3",
        "# idle first\nminimumIdleHours = 12",
        "webSearchProvider = \"searxng\"",
    ] {
        assert!(text.contains(kept), "{kept:?} missing from:\n{text}");
    }
    assert!(recovered(directory.path()).is_empty());
    assert_eq!(
        AgentConfig::new(directory.path()).settings().unwrap(),
        written
    );
}

#[test]
fn an_invalid_value_takes_its_default_and_the_original_is_kept() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_SETTINGS_FILE);
    let original = "permissionMode = \"everything\"\nwebFetchUseJinaReader = true\nwebSearchProvider = \"searxng\"\ndisabledPlugins = [\"titles\", \" padded \"]\n";
    fs::write(&path, original).unwrap();

    let config = AgentConfig::new(directory.path());
    let settings = config.settings().unwrap();

    assert_eq!(settings.permission_mode, AgentPermissionMode::Manual);
    assert!(settings.web_fetch_use_jina_reader);
    let recoveries = config.recoveries();
    assert_eq!(recoveries.len(), 1);
    assert_eq!(recoveries[0].file, path);
    assert_eq!(recoveries[0].read_only, None);
    assert!(matches!(
        &recoveries[0].problem,
        AgentConfigProblem::Repaired { repairs } if repairs.len() == 3
    ));
    assert_eq!(settings.web_search_provider, AgentWebSearchProvider::Auto);
    assert_eq!(
        settings.disabled_plugins.into_iter().collect::<Vec<_>>(),
        ["titles"]
    );
    assert_eq!(recovered(directory.path()), [original]);
    let repaired = AgentConfig::new(directory.path()).settings().unwrap();
    assert_eq!(repaired.permission_mode, AgentPermissionMode::Manual);
    assert_eq!(recovered(directory.path()).len(), 1);
}

#[test]
fn a_file_that_is_not_toml_is_kept_and_replaced_by_the_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_PROVIDERS_FILE);
    fs::write(&path, "revision = = 3").unwrap();

    let registry = AgentConfig::new(directory.path()).providers().unwrap();

    assert_eq!(registry, AgentProviderRegistrySettings::default());
    assert_eq!(recovered(directory.path()), ["revision = = 3"]);
    assert!(toml::from_str::<toml::Table>(&fs::read_to_string(&path).unwrap()).is_ok());
}

#[test]
fn the_provider_registry_is_revision_gated_and_owner_only() {
    let directory = tempfile::tempdir().unwrap();
    let config = AgentConfig::new(directory.path());
    let saved = config
        .mutate_providers(0, |registry| {
            registry.providers.push(provider("example"));
            registry.default_provider_id = Some("example".into());
            Ok(())
        })
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(
        config.mutate_providers(0, |_| Ok(())).unwrap_err(),
        "stale Agent Provider registry revision"
    );
    // A Provider that cannot be used is refused, and nothing is written.
    assert!(config
        .mutate_providers(1, |registry| {
            registry.providers[0].base_url = "ftp://example.test".into();
            Ok(())
        })
        .is_err());

    let reread = AgentConfig::new(directory.path()).providers().unwrap();
    assert_eq!(reread, saved);
    assert_eq!(
        reread.providers[0].api_token.as_deref(),
        Some("sk-verbatim token")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(directory.path().join(AGENT_PROVIDERS_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn a_provider_edit_and_a_settings_edit_do_not_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let config = AgentConfig::new(directory.path());
    let revision = config.providers().unwrap().revision;
    config
        .write_settings(AgentSettingsPatch {
            permission_mode: Some(AgentPermissionMode::FullAccess),
            ..AgentSettingsPatch::default()
        })
        .unwrap();
    config.set_plugin_disabled("titles", true).unwrap();
    config
        .mutate_providers(revision, |registry| {
            registry.providers.push(provider("example"));
            Ok(())
        })
        .unwrap();
    let settings = AgentConfig::new(directory.path()).settings().unwrap();
    assert_eq!(settings.permission_mode, AgentPermissionMode::FullAccess);
    assert!(settings.disabled_plugins.contains("titles"));
}

#[test]
fn mcp_settings_round_trip_through_toml_revision_gated_and_owner_only() {
    let directory = tempfile::tempdir().unwrap();
    let store = AgentConfig::new(directory.path()).mcp().clone();
    let saved = store
        .update(1, |settings| {
            settings.servers.push(stdio_server());
            Ok(())
        })
        .unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(
        store.update(1, |_| Ok(())).unwrap_err(),
        McpError::StaleGeneration
    );
    // A write is not repaired: an out-of-range timeout is refused.
    assert!(store
        .update(2, |settings| {
            settings.servers[0].connect_timeout_seconds = 0;
            Ok(())
        })
        .is_err());

    let reread = AgentConfig::new(directory.path()).mcp().clone();
    assert_eq!(reread.load().unwrap(), saved);
    assert!(reread.repairs().is_empty());
    let names = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, [AGENT_MCP_FILE]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(directory.path().join(AGENT_MCP_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn mcp_settings_that_cannot_be_read_are_kept_and_reported_as_a_reset() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_MCP_FILE);
    let store = McpSettingsStore::new(path.clone());

    fs::write(&path, "servers = [").unwrap();
    assert_eq!(store.load().unwrap(), McpSettings::default());
    fs::write(&path, vec![b' '; 8 * 1024 * 1024 + 1]).unwrap();
    assert_eq!(store.load().unwrap(), McpSettings::default());

    let repairs = store.repairs();
    assert_eq!(repairs.len(), 2);
    assert!(repairs
        .iter()
        .all(|repair| repair.kind == McpSettingsRepairKind::Reset));
    assert_eq!(recovered(directory.path()).len(), 2);
}

#[test]
fn mcp_repairs_keep_the_original_and_the_next_read_is_clean() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_MCP_FILE);
    let store = McpSettingsStore::new(path.clone());
    store
        .update(1, |settings| {
            settings.servers.push(stdio_server());
            Ok(())
        })
        .unwrap();
    let original = fs::read_to_string(&path)
        .unwrap()
        .replace("callTimeoutSeconds = 30", "callTimeoutSeconds = 0");
    fs::write(&path, &original).unwrap();

    let fresh = McpSettingsStore::new(path.clone());
    let settings = fresh.load().unwrap();

    assert_eq!(settings.servers[0].call_timeout_seconds, 1);
    assert_eq!(fresh.repairs().len(), 1);
    assert_eq!(recovered(directory.path()), [original]);
    assert_eq!(McpSettingsStore::new(path).load().unwrap(), settings);
    assert_eq!(recovered(directory.path()).len(), 1);
}

#[test]
fn a_removed_mcp_credential_leaves_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let store = McpSettingsStore::new(directory.path().join(AGENT_MCP_FILE));
    let credential = |value: &str| McpCredentialSettings {
        value: value.into(),
        generation: 1,
    };
    let kept = McpCredentialKey::parse("kept").unwrap();
    let removed = McpCredentialKey::parse("removed").unwrap();
    store
        .update(1, |settings| {
            settings
                .credentials
                .insert(kept.clone(), credential("kept-value"));
            settings
                .credentials
                .insert(removed.clone(), credential("removed-value"));
            Ok(())
        })
        .unwrap();

    store
        .update(2, |settings| {
            settings.credentials.remove(&removed);
            Ok(())
        })
        .unwrap();

    let settings = McpSettingsStore::new(store.path().to_path_buf())
        .load()
        .unwrap();
    assert_eq!(settings.credentials.keys().collect::<Vec<_>>(), [&kept]);
    assert!(!fs::read_to_string(store.path())
        .unwrap()
        .contains("removed-value"));
}

/// A regular file where `.recovery/` belongs, so keeping an original fails
/// on every system and for every user.
fn block_recovery(directory: &Path) {
    fs::write(directory.join(".recovery"), "not a directory").unwrap();
}

#[test]
fn a_repair_that_cannot_keep_the_original_leaves_the_file_read_only() {
    let directory = tempfile::tempdir().unwrap();
    block_recovery(directory.path());
    let path = directory.path().join(AGENT_SETTINGS_FILE);
    let original = "permissionMode = \"everything\"\nwebFetchUseJinaReader = true\n";
    fs::write(&path, original).unwrap();
    let config = AgentConfig::new(directory.path());

    let settings = config.settings().unwrap();

    assert_eq!(settings.permission_mode, AgentPermissionMode::Manual);
    assert!(settings.web_fetch_use_jina_reader);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    let error = config
        .write_settings(AgentSettingsPatch {
            web_fetch_use_jina_reader: Some(false),
            ..AgentSettingsPatch::default()
        })
        .unwrap_err();
    assert!(
        error.contains("read-only until the Agent restarts"),
        "{error}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    let recoveries = config.recoveries();
    assert_eq!(recoveries.len(), 1, "a read-only file is recorded once");
    assert!(recoveries[0].read_only.is_some());
    // The other files are not affected.
    config
        .mutate_providers(0, |registry| {
            registry.providers.push(provider("example"));
            Ok(())
        })
        .unwrap();
}

#[test]
fn unusable_mcp_settings_that_cannot_be_kept_stay_on_disk_and_refuse_writes() {
    let directory = tempfile::tempdir().unwrap();
    block_recovery(directory.path());
    let path = directory.path().join(AGENT_MCP_FILE);
    fs::write(&path, "servers = [").unwrap();
    let store = McpSettingsStore::new(path.clone());

    assert_eq!(store.load().unwrap(), McpSettings::default());
    assert_eq!(store.load().unwrap(), McpSettings::default());

    assert_eq!(store.repairs().len(), 1);
    assert!(matches!(
        store.update(1, |_| Ok(())),
        Err(McpError::Unavailable(error)) if error.contains("read-only")
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "servers = [");
}

#[test]
fn a_new_file_lists_the_keys_in_their_declared_order() {
    let directory = tempfile::tempdir().unwrap();
    let config = AgentConfig::new(directory.path());

    config
        .write_settings(AgentSettingsPatch::default())
        .unwrap();

    let text = fs::read_to_string(directory.path().join(AGENT_SETTINGS_FILE)).unwrap();
    let position = |key: &str| {
        text.find(key)
            .unwrap_or_else(|| panic!("{key} in:\n{text}"))
    };
    // Sorted, disabledPlugins would come first.
    assert!(
        position("permissionMode") < position("disabledPlugins"),
        "{text}"
    );
    assert!(
        position("disabledPlugins") < position("webSearchProvider"),
        "{text}"
    );
    assert_eq!(
        AgentConfig::new(directory.path()).settings().unwrap(),
        AgentSettings::default()
    );
}

#[test]
fn the_sandbox_settings_read_tolerantly_and_an_unset_mode_is_never_written() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_SETTINGS_FILE);
    let config = AgentConfig::new(directory.path());
    assert_eq!(config.settings().unwrap().sandbox_mode, None);
    config
        .write_settings(AgentSettingsPatch {
            permission_mode: Some(AgentPermissionMode::AutoReview),
            ..AgentSettingsPatch::default()
        })
        .unwrap();
    assert!(!fs::read_to_string(&path).unwrap().contains("sandboxMode"));

    // Absolute on every platform: `/data` is not on Windows. A TOML literal
    // string, so a Windows path's backslashes are not escapes.
    let root = directory.path().join("data");
    let root_text = root.to_str().unwrap();
    fs::write(
        &path,
        format!(
            "sandboxMode = \"workspace-write\"\n[sandboxWorkspaceWrite]\nwritableRoots = ['{root_text}']\nnetworkAccess = true\n"
        ),
    )
    .unwrap();
    let settings = AgentConfig::new(directory.path()).settings().unwrap();
    assert_eq!(
        settings.sandbox_mode,
        Some(AgentSandboxMode::WorkspaceWrite)
    );
    assert_eq!(
        settings.sandbox_workspace_write,
        AgentSandboxWorkspaceWrite {
            writable_roots: vec![root.clone()],
            network_access: true,
        }
    );

    // An unreadable mode is the most restrictive; a relative root drops the
    // list, not the network switch beside it.
    fs::write(
        &path,
        format!(
            "sandboxMode = \"sandboxed\"\n[sandboxWorkspaceWrite]\nwritableRoots = ['{root_text}', \"relative\"]\nnetworkAccess = true\n"
        ),
    )
    .unwrap();
    let config = AgentConfig::new(directory.path());
    let settings = config.settings().unwrap();
    assert_eq!(settings.sandbox_mode, Some(AgentSandboxMode::ReadOnly));
    assert!(settings.sandbox_workspace_write.writable_roots.is_empty());
    assert!(settings.sandbox_workspace_write.network_access);
    assert!(matches!(
        &config.recoveries()[0].problem,
        AgentConfigProblem::Repaired { repairs } if repairs.len() == 2
    ));

    let refused = config.write_settings(AgentSettingsPatch {
        sandbox_workspace_write: Some(AgentSandboxWorkspaceWrite {
            writable_roots: vec!["relative".into()],
            network_access: false,
        }),
        ..AgentSettingsPatch::default()
    });
    assert!(refused.is_err());
    let reset: AgentSettingsPatch = serde_json::from_value(json!({"sandboxMode": null})).unwrap();
    assert_eq!(config.write_settings(reset).unwrap().sandbox_mode, None);
}

#[test]
fn an_unreadable_windows_sandbox_is_dropped_which_turns_it_off() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_SETTINGS_FILE);
    fs::write(&path, "[windows]\nsandbox = \"unelevated\"\n").unwrap();
    let config = AgentConfig::new(directory.path());
    assert_eq!(
        config.settings().unwrap().windows.sandbox,
        Some(crate::AgentWindowsSandbox::Unelevated)
    );

    fs::write(&path, "[windows]\nsandbox = \"admin\"\n").unwrap();
    let config = AgentConfig::new(directory.path());
    assert_eq!(config.settings().unwrap().windows.sandbox, None);
    assert!(matches!(
        &config.recoveries()[0].problem,
        AgentConfigProblem::Repaired { repairs } if repairs.len() == 1
    ));

    fs::remove_file(&path).unwrap();
    let config = AgentConfig::new(directory.path());
    config
        .write_settings(AgentSettingsPatch {
            permission_mode: Some(AgentPermissionMode::AutoReview),
            ..AgentSettingsPatch::default()
        })
        .unwrap();
    let written = fs::read_to_string(&path).unwrap();
    assert!(!written.contains("windows"), "{written}");
}

#[test]
fn model_discovery_reads_tolerantly_and_is_set_by_its_key() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_SETTINGS_FILE);
    let discovery = |config: &AgentConfig| config.settings().unwrap().model_metadata.discovery;
    assert_eq!(
        discovery(&AgentConfig::new(directory.path())),
        AgentModelDiscovery::Auto
    );

    // A key a later version adds is ignored, and the one beside it read.
    fs::write(
        &path,
        "[modelMetadata]\ndiscovery = \"off\"\nmodelsDev = true\n",
    )
    .unwrap();
    let config = AgentConfig::new(directory.path());
    assert_eq!(discovery(&config), AgentModelDiscovery::Off);
    assert!(config.recoveries().is_empty());

    for unusable in [
        "[modelMetadata]\ndiscovery = \"sometimes\"\n",
        "modelMetadata = 3\n",
    ] {
        fs::write(&path, unusable).unwrap();
        let config = AgentConfig::new(directory.path());
        assert_eq!(discovery(&config), AgentModelDiscovery::Auto, "{unusable}");
        assert!(matches!(
            &config.recoveries()[0].problem,
            AgentConfigProblem::Repaired { repairs } if repairs.len() == 1
        ));
    }

    fs::remove_file(&path).unwrap();
    let config = AgentConfig::new(directory.path());
    config
        .set_user_value("modelMetadata.discovery", "off")
        .unwrap();
    assert_eq!(discovery(&config), AgentModelDiscovery::Off);
    assert!(config
        .set_user_value("modelMetadata.discovery", "sometimes")
        .is_err());
    assert_eq!(
        config.value("modelMetadata.discovery").unwrap(),
        Some(json!("off"))
    );
}

/// Per-model overrides read field by field: a field that cannot be used is
/// unset (an unreadable `reasoningEfforts` unset, never "no levels"), an
/// entry that is not a table is dropped, and the Provider and its token
/// stay; a write keeps the overrides, ids with `/` and `.` included.
#[test]
fn per_model_overrides_read_field_by_field_and_survive_a_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(AGENT_PROVIDERS_FILE);
    fs::write(
        &path,
        r#"revision = 1
[[providers]]
id = "provider-1"
displayName = "Example"
enabled = true
protocol = "responses"
baseUrl = "https://api.example.test/v1"
apiToken = "sk-kept"

[providers.models."openai/gpt-4.1"]
contextWindowTokens = 0
maxOutputTokens = 32000
imageInput = "supported"
reasoningEfforts = ["low", "none"]

[providers.models."claude-3.5"]
contextWindowTokens = 200000
imageInput = "unsupported"
reasoningEfforts = []

[providers.models]
broken = 3
"#,
    )
    .unwrap();

    let config = AgentConfig::new(directory.path());
    let registry = config.providers().unwrap();

    let provider = &registry.providers[0];
    assert_eq!(provider.api_token.as_deref(), Some("sk-kept"));
    let expected = BTreeMap::from([
        (
            "claude-3.5".to_string(),
            AgentModelOverrideSettings {
                context_window_tokens: Some(200_000),
                max_output_tokens: None,
                image_input: AgentModelImageInput::Unsupported,
                reasoning_efforts: Some(Vec::new()),
            },
        ),
        (
            "openai/gpt-4.1".to_string(),
            AgentModelOverrideSettings {
                context_window_tokens: None,
                max_output_tokens: Some(32_000),
                image_input: AgentModelImageInput::Supported,
                reasoning_efforts: None,
            },
        ),
    ]);
    assert_eq!(provider.models, expected);
    assert!(matches!(
        &config.recoveries()[0].problem,
        AgentConfigProblem::Repaired { repairs } if repairs.len() == 3
    ));

    let saved = config
        .mutate_providers(1, |registry| {
            registry.providers[0].display_name = "Renamed".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(saved.providers[0].models, expected);
    let reopened = AgentConfig::new(directory.path());
    assert_eq!(reopened.providers().unwrap().providers[0].models, expected);
    assert!(
        reopened.recoveries().is_empty(),
        "the written file reads clean"
    );
}
