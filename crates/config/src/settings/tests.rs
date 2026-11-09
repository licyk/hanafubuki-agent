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

#[test]
fn compaction_fields_fall_back_one_by_one() {
    let object = serde_json::json!({
        "compaction": {
            "strategy": "hierarchical",
            "background": {"thresholdPercent": 0, "childSessions": false},
            "hierarchical": {
                "maxContextLimitPercent": 70,
                "emergencyThresholdPercent": 101,
                "minSummaryLength": 30000,
                "maxSummaryLength": 40000,
                "protectedTools": ["shell"],
                "absorb": true,
                "unknown": 1
            }
        }
    });
    let mut repairs = Vec::new();
    let settings = decode_settings(
        object.as_object().unwrap(),
        &AgentSettings::default(),
        &mut repairs,
    );
    let compaction = settings.compaction;
    assert_eq!(compaction.strategy, AgentCompactionStrategy::Hierarchical);
    assert_eq!(compaction.background.threshold_percent, 65);
    assert!(!compaction.background.child_sessions);
    let hierarchical = compaction.hierarchical;
    assert_eq!(hierarchical.max_context_limit_percent, 70);
    assert_eq!(hierarchical.emergency_threshold_percent, 95);
    // Read in either order, a pair that holds is kept.
    assert_eq!(hierarchical.min_summary_length, 30000);
    assert_eq!(hierarchical.max_summary_length, 40000);
    assert_eq!(hierarchical.protected_tools, vec!["shell".to_string()]);
    assert!(hierarchical.absorb);
    assert_eq!(
        repairs,
        vec![
            "compaction.background.thresholdPercent: the value is invalid and was replaced with its default",
            "compaction.hierarchical.emergencyThresholdPercent: the value is invalid and was replaced with its default",
        ]
    );

    let object = serde_json::json!({
        "compaction": {
            "strategy": "eventually",
            "hierarchical": {"minSummaryLength": 500, "maxSummaryLength": 100}
        }
    });
    let mut repairs = Vec::new();
    let settings = decode_settings(
        object.as_object().unwrap(),
        &AgentSettings::default(),
        &mut repairs,
    );
    assert_eq!(settings.compaction, AgentCompactionSettings::default());
    assert_eq!(repairs.len(), 2, "{repairs:?}");
    assert!(repairs[1].contains("minSummaryLength is above maxSummaryLength"));
}

#[test]
fn a_compaction_patch_replaces_the_table_once_it_holds() {
    let mut settings = AgentSettings::default();
    patch(serde_json::json!({"compaction": {
        "strategy": "background",
        "background": {"thresholdPercent": 50, "childSessions": false},
        "hierarchical": AgentHierarchicalCompactionSettings::default()
    }}))
    .unwrap()
    .apply(&mut settings)
    .unwrap();
    assert_eq!(
        settings.compaction.strategy,
        AgentCompactionStrategy::Background
    );
    assert_eq!(settings.compaction.background.threshold_percent, 50);

    let mut crossed = AgentCompactionSettings::default();
    crossed.hierarchical.min_summary_length = 1_000_000;
    assert!(AgentSettingsPatch {
        compaction: Some(crossed),
        ..AgentSettingsPatch::default()
    }
    .apply(&mut settings)
    .is_err());
    assert!(
        patch(serde_json::json!({"compaction": {"strategy": "standard", "extra": 1}})).is_err()
    );
}
