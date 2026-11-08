use super::*;

fn patch(value: serde_json::Value) -> Result<AgentSettingsPatch, serde_json::Error> {
    serde_json::from_value(value)
}

#[test]
fn a_patch_replaces_only_what_it_names() {
    let mut settings = AgentSettings {
        searxng_base_url: Some("https://search.example.test".into()),
        ..AgentSettings::default()
    };
    patch(serde_json::json!({"permissionMode": "full_access", "webFetchUseJinaReader": true}))
        .unwrap()
        .apply(&mut settings)
        .unwrap();
    assert_eq!(settings.permission_mode, AgentPermissionMode::FullAccess);
    assert!(settings.web_fetch_use_jina_reader);
    assert_eq!(
        settings.searxng_base_url.as_deref(),
        Some("https://search.example.test")
    );

    patch(serde_json::json!({"searxngBaseUrl": null}))
        .unwrap()
        .apply(&mut settings)
        .unwrap();
    assert_eq!(settings.searxng_base_url, None);
}

#[test]
fn a_patch_is_checked_as_a_whole() {
    let mut settings = AgentSettings::default();
    let error = patch(serde_json::json!({"webSearchProvider": "searxng"}))
        .unwrap()
        .apply(&mut settings.clone())
        .unwrap_err();
    assert!(error.contains("searxngBaseUrl is required"));
    for url in [
        "   ",
        "https://user:secret@search.example.test",
        "not a url",
    ] {
        assert!(patch(serde_json::json!({"searxngBaseUrl": url}))
            .unwrap()
            .apply(&mut settings)
            .is_err());
    }
    let memory = AgentMemorySettings {
        max_sessions_per_pass: 0,
        ..AgentMemorySettings::default()
    };
    assert!(AgentSettingsPatch {
        memory: Some(memory),
        ..AgentSettingsPatch::default()
    }
    .apply(&mut settings)
    .is_err());
    assert_eq!(settings, AgentSettings::default());
}

#[test]
fn a_patch_refuses_a_key_it_does_not_know() {
    assert!(patch(serde_json::json!({"disabledPlugins": ["titles"]})).is_err());
    assert!(patch(serde_json::json!({"agentPermissionMode": "manual"})).is_err());
}
