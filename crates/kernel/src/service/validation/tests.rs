use super::persistence::validate_turn_boundaries;
use super::tool_pairs::interrupted_tool_output;
use super::*;
use crate::AgentCompactionCheckpoint;

fn item(value: serde_json::Value) -> ConversationItem {
    serde_json::from_value(value).unwrap()
}

#[test]
fn prompt_projection_moves_a_late_interruption_output_next_to_its_call() {
    let call = item(json!({
        "type": "tool_call",
        "callId": "call-1",
        "name": "list_instances",
        "catalogRevision": vec![0; 32],
        "providerId": "builtin",
        "toolId": "list_instances",
        "toolGeneration": vec![0; 32],
        "arguments": {}
    }));
    let later_message = item(json!({
        "type": "user_message",
        "text": "continue after the failed tool"
    }));
    let output = interrupted_tool_output(
        "call-1".to_string(),
        hanafubuki_agent_model_api::ModelToolCallOrigin::Function,
    );

    let normalized =
        normalized_tool_pairs_for_model(&[call.clone(), later_message.clone(), output.clone()])
            .unwrap();

    assert_eq!(normalized, vec![call, output, later_message]);
}

#[test]
fn prompt_projection_inserts_an_output_for_an_unmatched_call() {
    let call = item(json!({
        "type": "tool_call",
        "callId": "call-1",
        "name": "list_instances",
        "catalogRevision": vec![0; 32],
        "providerId": "builtin",
        "toolId": "list_instances",
        "toolGeneration": vec![0; 32],
        "arguments": {}
    }));
    let later_message = item(json!({
        "type": "user_message",
        "text": "continue after the failed tool"
    }));

    let normalized =
        normalized_tool_pairs_for_model(&[call.clone(), later_message.clone()]).unwrap();

    assert!(matches!(
        &normalized[1],
        ConversationItem::ToolOutput { call_id, output, .. }
            if call_id == "call-1" && output.is_error
    ));
    assert_eq!(normalized[0], call);
    assert_eq!(normalized[2], later_message);
}

#[test]
fn freeform_calls_and_outputs_replay_as_custom_responses_items() {
    let patch = "*** Begin Patch\n*** Add File: note.txt\n+hello\n*** End Patch";
    let call = item(json!({
        "type": "tool_call",
        "callId": "patch-1",
        "name": "apply_patch",
        "origin": "freeform",
        "catalogRevision": vec![0; 32],
        "providerId": "builtin",
        "toolId": "apply_patch",
        "toolGeneration": vec![0; 32],
        "arguments": patch
    }));
    let output = ConversationItem::ToolOutput {
        call_id: "patch-1".to_string(),
        origin: hanafubuki_agent_model_api::ModelToolCallOrigin::Freeform,
        output: hanafubuki_agent_tool_contracts::ToolOutput::structured(
            json!({"status": "ok"}),
            false,
        ),
        truncation: None,
        images: Vec::new(),
        tool_search_results: Vec::new(),
    };

    assert!(matches!(
        call.as_model_input(),
        Some(hanafubuki_agent_model_api::ModelInputItem::CustomToolCall { call_id, name, input })
            if call_id == "patch-1" && name == "apply_patch" && input == patch
    ));
    assert!(matches!(
        output.as_model_input(),
        Some(hanafubuki_agent_model_api::ModelInputItem::CustomToolOutput { call_id, .. })
            if call_id == "patch-1"
    ));
}

#[test]
fn implicit_skill_context_must_immediately_follow_its_tool_output() {
    let turn = item(json!({ "type": "turn_start", "turnId": 1 }));
    let user = item(json!({ "type": "user_message", "text": "use a skill" }));
    let use_call = item(json!({
        "type": "tool_call",
        "callId": "call-skill",
        "name": "read_guidance",
        "catalogRevision": vec![0; 32],
        "providerId": "builtin",
        "toolId": "read_guidance",
        "toolGeneration": vec![0; 32],
        "arguments": {}
    }));
    let output = interrupted_tool_output(
        "call-skill".to_string(),
        hanafubuki_agent_model_api::ModelToolCallOrigin::Function,
    );
    let context = ConversationItem::SkillContext {
        skill: hanafubuki_agent_skills::SkillId::new(
            hanafubuki_agent_skills::SkillScope::User,
            "workflow",
        )
        .unwrap(),
        name: "Workflow".to_string(),
        content: "<skill>\nguidance\n</skill>".to_string(),
    };
    assert!(validate_turn_boundaries(&[
        turn.clone(),
        user.clone(),
        use_call,
        output.clone(),
        context.clone(),
    ])
    .is_ok());
    assert!(validate_turn_boundaries(&[turn, user, output, context]).is_err());
}

#[test]
fn skills_read_output_and_context_round_trip_as_a_valid_persisted_group() {
    let skill = hanafubuki_agent_skills::SkillId::new(
        hanafubuki_agent_skills::SkillScope::User,
        "workflow",
    )
    .unwrap();
    let history = vec![
        item(json!({ "type": "turn_start", "turnId": 1 })),
        item(json!({ "type": "user_message", "text": "use a skill" })),
        item(json!({
            "type": "tool_call",
            "callId": "call-skill",
            "name": "skills_read",
            "catalogRevision": vec![0; 32],
            "providerId": "builtin",
            "toolId": "skills_read",
            "toolGeneration": vec![0; 32],
            "arguments": {}
        })),
        interrupted_tool_output(
            "call-skill".to_string(),
            hanafubuki_agent_model_api::ModelToolCallOrigin::Function,
        ),
        ConversationItem::SkillContext {
            skill: skill.clone(),
            name: "Workflow".to_string(),
            content: "<skill>\ncanonical guidance\n</skill>".to_string(),
        },
    ];
    let encoded = serde_json::to_vec(&history).unwrap();
    let reconstructed: Vec<ConversationItem> = serde_json::from_slice(&encoded).unwrap();
    let config = AgentSessionConfig {
        provider_id: "provider-1".to_string(),
        provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol::Responses,
        model: "fixture".to_string(),
        instructions: "fixture".to_string(),
        mode: crate::AgentSessionMode::Default,
        reasoning_effort: None,
        context_window_tokens: None,
        input_capabilities: Default::default(),
        auto_compaction_enabled: true,
        auto_compact_threshold_percent: 80,
        compaction_capability: hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary,
        sampling_retry_policy: AgentSamplingRetryPolicy::default(),
        context_window_source: None,
        compaction_strategy: Default::default(),
        scope: Default::default(),
        role_instructions: None,
    };

    validate_persisted_history(&reconstructed, &config, crate::FIRST_BATCH_HARD_LIMITS).unwrap();
    assert!(matches!(
        reconstructed.as_slice(),
        [
            ..,
            ConversationItem::ToolOutput { .. },
            ConversationItem::SkillContext { .. }
        ]
    ));
}

fn valid_window_checkpoint() -> (
    AgentContextWindowSnapshot,
    AgentCompactionSnapshot,
    AgentModelContextBoundary,
) {
    let source_window = AgentContextWindowSnapshot::initial();
    let target_window = source_window.next().unwrap();
    let snapshot = AgentCompactionSnapshot {
        id: AgentCompactionId::new(1),
        generation: AgentCompactionGeneration::new(1),
        trigger: AgentCompactionTrigger::Manual,
        phase: AgentCompactionPhase::StandaloneTurn,
        reason: AgentCompactionReason::UserRequested,
        implementation: AgentCompactionImplementation::LocalSummary,
        status: AgentCompactionStatus::Completed,
        source_revision: AgentRevision::new(4),
        source_window_id: source_window.window_id,
        target_window,
        turn_generation: None,
        summary: Some("summary".to_string()),
        usage: None,
        last_error: None,
        created_at: 1,
        updated_at: 2,
    };
    let boundary = AgentModelContextBoundary::Compacted {
        checkpoint: AgentCompactionCheckpoint {
            checkpoint_item_id: AgentHistoryItemId::new(2),
            snapshot: snapshot.clone(),
            replacement_history: vec![AgentHistoryItem {
                source_session_id: AgentSessionId::new(1),
                item_id: AgentHistoryItemId::new(1),
                item_revision: 0,
                turn_id: None,
                logical_segment: 0,
                logical_order: 1,
                turn_working_millis: None,
                content: ConversationItem::CompactionSummary {
                    summary: "summary".to_string(),
                    source_revision: AgentRevision::new(4),
                    compacted_item_count: 1,
                },
            }],
            window_chain: vec![source_window, target_window],
            future_items_after_item_id: 2,
            initial_context_digest: [7; 32],
        },
    };
    (target_window, snapshot, boundary)
}

#[test]
fn recovery_rejects_duplicate_regressed_and_wrong_previous_window_links() {
    let (window, snapshot, boundary) = valid_window_checkpoint();
    validate_context_window_checkpoint(
        window,
        Some(&snapshot),
        &boundary,
        AgentRevision::new(8),
        crate::FIRST_BATCH_HARD_LIMITS,
    )
    .unwrap();

    for mutation in ["duplicate", "regressed", "wrong_previous"] {
        let mut invalid = boundary.clone();
        let AgentModelContextBoundary::Compacted { checkpoint } = &mut invalid else {
            unreachable!();
        };
        match mutation {
            "duplicate" => {
                checkpoint.window_chain[1].window_id = checkpoint.window_chain[0].window_id
            }
            "regressed" => checkpoint.window_chain[1].window_number = 0,
            "wrong_previous" => {
                checkpoint.window_chain[1].previous_window_id =
                    Some(checkpoint.window_chain[1].window_id)
            }
            _ => unreachable!(),
        }
        assert!(validate_context_window_checkpoint(
            window,
            Some(&snapshot),
            &invalid,
            AgentRevision::new(8),
            crate::FIRST_BATCH_HARD_LIMITS,
        )
        .is_err());
    }
}

#[test]
fn checkpoint_accepts_live_window_with_backfilled_prefill_tokens() {
    let (window, snapshot, boundary) = valid_window_checkpoint();
    let mut live = window;
    live.prefill_input_tokens = Some(1035);
    let mut invalid = boundary.clone();
    let AgentModelContextBoundary::Compacted { checkpoint } = &mut invalid else {
        unreachable!();
    };
    assert!(checkpoint
        .snapshot
        .target_window
        .prefill_input_tokens
        .is_none());
    validate_context_window_checkpoint(
        live,
        Some(&snapshot),
        &invalid,
        AgentRevision::new(8),
        crate::FIRST_BATCH_HARD_LIMITS,
    )
    .unwrap();
}

fn responses_only_config(
    protocol: hanafubuki_agent_model_api::ModelApiProtocol,
) -> AgentSessionConfig {
    AgentSessionConfig {
        provider_id: "provider-1".to_string(),
        provider_protocol: protocol,
        model: "fixture-model".to_string(),
        instructions: "fixture".to_string(),
        mode: crate::AgentSessionMode::Default,
        reasoning_effort: Some(hanafubuki_agent_model_api::ModelReasoningEffort::High),
        context_window_tokens: Some(128_000),
        input_capabilities: Default::default(),
        auto_compaction_enabled: true,
        auto_compact_threshold_percent: 70,
        compaction_capability:
            hanafubuki_agent_model_api::ModelCompactionCapability::ResponsesCompact,
        sampling_retry_policy: AgentSamplingRetryPolicy::default(),
        context_window_source: None,
        compaction_strategy: Default::default(),
        scope: Default::default(),
        role_instructions: None,
    }
}

/// The one rule for values only Responses carries: kept on Responses,
/// dropped elsewhere, with everything protocol-neutral untouched. The
/// application passes the shared policy through, so this covers what its
/// resolution used to narrow.
#[test]
fn only_a_responses_session_keeps_reasoning_effort_and_remote_compaction() {
    let mut responses =
        responses_only_config(hanafubuki_agent_model_api::ModelApiProtocol::Responses);
    normalize_session_config(&mut responses);
    assert_eq!(
        responses,
        responses_only_config(hanafubuki_agent_model_api::ModelApiProtocol::Responses)
    );
    validate_session_config(&responses, crate::FIRST_BATCH_HARD_LIMITS).unwrap();

    for protocol in [
        hanafubuki_agent_model_api::ModelApiProtocol::ChatCompletions,
        hanafubuki_agent_model_api::ModelApiProtocol::AnthropicMessages,
    ] {
        let mut config = responses_only_config(protocol);
        normalize_session_config(&mut config);
        assert_eq!(config.reasoning_effort, None, "{protocol:?}");
        assert_eq!(
            config.compaction_capability,
            hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary,
            "{protocol:?}"
        );
        assert_eq!(config.context_window_tokens, Some(128_000), "{protocol:?}");
        assert!(config.auto_compaction_enabled, "{protocol:?}");
        assert_eq!(config.auto_compact_threshold_percent, 70, "{protocol:?}");
        validate_session_config(&config, crate::FIRST_BATCH_HARD_LIMITS).unwrap();
    }

    // An unconfigured session is neutral on every protocol.
    let mut neutral =
        responses_only_config(hanafubuki_agent_model_api::ModelApiProtocol::Responses);
    neutral.model.clear();
    neutral.instructions.clear();
    normalize_session_config(&mut neutral);
    assert_eq!(neutral.reasoning_effort, None);
    assert_eq!(
        neutral.compaction_capability,
        hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary
    );
    validate_session_config(&neutral, crate::FIRST_BATCH_HARD_LIMITS).unwrap();
}

/// The bound a host cuts its context to is the bound recovery holds
/// persisted steering to.
#[test]
fn a_host_context_is_accepted_up_to_the_bound_the_host_cuts_to() {
    let context = |content: String| crate::AgentHostContext {
        kind: hanafubuki_agent_contracts::ContextSectionKind::fixed("host_location"),
        content,
    };
    validate_host_context(&context("a".repeat(crate::MAXIMUM_HOST_CONTEXT_BYTES))).unwrap();
    assert!(matches!(
        validate_host_context(&context("a".repeat(crate::MAXIMUM_HOST_CONTEXT_BYTES + 1))),
        Err(AgentError::LimitExceeded { limit, .. })
            if limit == crate::MAXIMUM_HOST_CONTEXT_BYTES
    ));
    assert!(validate_host_context(&context(" ".to_string())).is_err());
}

/// A session without a role writes the record it wrote before roles
/// existed, and such a record reads with the empty scope; a role's scope
/// and instructions are kept, canonical, and requests carry its
/// instructions after the base ones.
#[test]
fn a_default_session_config_is_unchanged_and_a_role_survives_its_record() {
    let config = responses_only_config(hanafubuki_agent_model_api::ModelApiProtocol::Responses);
    let value = serde_json::to_value(&config).unwrap();
    assert!(value.get("scope").is_none() && value.get("roleInstructions").is_none());
    let read: AgentSessionConfig = serde_json::from_value(value).unwrap();
    assert!(read.scope.is_empty());
    assert_eq!(read.effective_instructions(), "fixture");

    let mut role = config.clone();
    role.instructions = "base\n".to_string();
    role.role_instructions = Some("Only read.".to_string());
    role.scope = crate::AgentCapabilityScope {
        role: Some("reader".to_string()),
        plugins_off: ["agent-kernel".to_string(), "titles".to_string()].into(),
        ..Default::default()
    };
    normalize_session_config(&mut role);
    assert_eq!(role.scope.plugins_off, ["titles".to_string()].into());
    let read: AgentSessionConfig =
        serde_json::from_value(serde_json::to_value(&role).unwrap()).unwrap();
    assert_eq!(read, role);
    assert_eq!(read.effective_instructions(), "base\n\nOnly read.\n");
    // A field this build cannot read falls back alone.
    let mut value = serde_json::to_value(&role).unwrap();
    value["roleInstructions"] = serde_json::json!(5);
    let read: AgentSessionConfig = serde_json::from_value(value).unwrap();
    assert_eq!((read.role_instructions, read.scope), (None, role.scope));
}

/// A window's source is written only while it is known; one this build
/// cannot read is none (the window then counts as configured), and
/// normalization drops a source the window lost.
#[test]
fn a_context_window_source_reads_tolerantly_and_goes_with_its_window() {
    use hanafubuki_agent_model_api::ModelMetadataSource;
    let config = responses_only_config(hanafubuki_agent_model_api::ModelApiProtocol::Responses);
    let value = serde_json::to_value(&config).unwrap();
    assert!(value.get("contextWindowSource").is_none());

    let mut reported = config.clone();
    reported.context_window_source = Some(ModelMetadataSource::Reported);
    let mut value = serde_json::to_value(&reported).unwrap();
    assert_eq!(value["contextWindowSource"], "reported");
    let read: AgentSessionConfig = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(read, reported);
    value["contextWindowSource"] = serde_json::json!("models_dev");
    let read: AgentSessionConfig = serde_json::from_value(value).unwrap();
    assert_eq!(read.context_window_source, None);
    assert_eq!(
        read.context_window_origin(),
        Some(ModelMetadataSource::Configured)
    );

    reported.context_window_tokens = Some(0);
    normalize_session_config(&mut reported);
    assert_eq!(
        (
            reported.context_window_tokens,
            reported.context_window_source
        ),
        (None, None)
    );
}
