use super::*;

fn plan_message() -> ConversationItem {
    ConversationItem::AssistantMessage {
        message_id: AgentMessageId::new(7),
        phase: Some(AgentMessagePhase::FinalAnswer),
        text: "Visible introduction.\n".to_string(),
        model_text: Some(
            "Visible introduction.\n<proposed_plan>\n- Change it.\n</proposed_plan>".to_string(),
        ),
        proposed_plan: Some(AgentPlanBlock {
            text: "- Change it.\n".to_string(),
        }),
        sampling_step: 2,
        memory_citations: Vec::new(),
    }
}

#[test]
fn plan_message_round_trips_without_merging_model_and_presentation_text() {
    let item = plan_message();
    let encoded = serde_json::to_value(&item).unwrap();
    assert_eq!(encoded["text"], "Visible introduction.\n");
    assert_eq!(encoded["proposedPlan"]["text"], "- Change it.\n");
    assert!(encoded["modelText"]
        .as_str()
        .unwrap()
        .contains("<proposed_plan>"));

    assert_eq!(
        serde_json::from_value::<ConversationItem>(encoded).unwrap(),
        item
    );
}

#[test]
fn model_replay_uses_canonical_text_while_plain_messages_use_visible_text() {
    assert!(matches!(
        plan_message().as_model_input(),
        Some(hanafubuki_agent_model_api::ModelInputItem::AssistantMessage { text, .. })
            if text.contains("<proposed_plan>") && text.contains("Change it")
    ));

    let plain = ConversationItem::AssistantMessage {
        message_id: AgentMessageId::new(8),
        phase: None,
        text: "plain".to_string(),
        model_text: None,
        proposed_plan: None,
        sampling_step: 1,
        memory_citations: Vec::new(),
    };
    assert!(matches!(
        plain.as_model_input(),
        Some(hanafubuki_agent_model_api::ModelInputItem::AssistantMessage { text, .. }) if text == "plain"
    ));
}

#[test]
fn transcript_serialization_hides_canonical_text_but_keeps_the_typed_plan() {
    let page = crate::AgentTranscriptPage {
        items: vec![crate::AgentHistoryItem {
            source_session_id: AgentSessionId::new(1),
            item_id: crate::AgentHistoryItemId::new(1),
            item_revision: 0,
            turn_id: Some(AgentTurnId::new(1)),
            logical_segment: 0,
            logical_order: 0,
            turn_working_millis: None,
            content: plan_message(),
        }],
        older_cursor: None,
        newer_cursor: None,
        through_revision: AgentRevision::new(1),
    };
    let encoded = serde_json::to_value(page).unwrap();

    assert!(encoded["items"][0]["content"].get("modelText").is_none());
    assert_eq!(
        encoded["items"][0]["content"]["proposedPlan"]["text"],
        "- Change it.\n"
    );
}

fn reasoning(replay: Option<hanafubuki_agent_model_api::ModelReasoningReplay>) -> ConversationItem {
    ConversationItem::Reasoning {
        text: "Shown thought.".to_string(),
        duration_millis: 5,
        sampling_step: 1,
        truncated: false,
        replay,
    }
}

#[test]
fn reasoning_content_is_persisted_and_anthropics_blocks_are_not() {
    use hanafubuki_agent_model_api::ModelReasoningReplay;
    let sent_back = reasoning(Some(ModelReasoningReplay::ReasoningContent {
        text: "Whole thought.".to_string(),
    }));
    let encoded = serde_json::to_value(&sent_back).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({
            "type": "reasoning",
            "text": "Shown thought.",
            "durationMillis": 5,
            "samplingStep": 1,
            "truncated": false,
            "replay": {"type": "reasoning_content", "text": "Whole thought."}
        })
    );
    assert_eq!(
        serde_json::from_value::<ConversationItem>(encoded).unwrap(),
        sent_back
    );

    for replay in [
        None,
        Some(ModelReasoningReplay::Thinking {
            thinking: "Signed.".to_string(),
            signature: "sig".to_string(),
        }),
        Some(ModelReasoningReplay::RedactedThinking {
            data: "opaque".to_string(),
        }),
    ] {
        let encoded = serde_json::to_value(reasoning(replay)).unwrap();
        assert!(encoded.get("replay").is_none(), "{encoded}");
    }
}

#[test]
fn a_persisted_replay_it_cannot_use_reads_as_none() {
    let oversized = "x".repeat(hanafubuki_agent_model_api::MAX_MODEL_REASONING_REPLAY_BYTES + 1);
    for replay in [
        serde_json::json!(null),
        serde_json::json!("Whole thought."),
        serde_json::json!({"type": "reasoning_content"}),
        serde_json::json!({"type": "future_kind", "text": "Whole thought."}),
        serde_json::json!({"type": "thinking", "thinking": "Signed.", "signature": "sig"}),
        serde_json::json!({"type": "reasoning_content", "text": oversized}),
    ] {
        let encoded = serde_json::json!({
            "type": "reasoning",
            "text": "Shown thought.",
            "samplingStep": 1,
            "replay": replay,
        });
        assert_eq!(
            serde_json::from_value::<ConversationItem>(encoded).unwrap(),
            ConversationItem::Reasoning {
                text: "Shown thought.".to_string(),
                duration_millis: 0,
                sampling_step: 1,
                truncated: false,
                replay: None,
            }
        );
    }
}

#[test]
fn a_transcript_leaves_the_reasoning_sent_back_out() {
    let page = crate::AgentTranscriptPage {
        items: vec![crate::AgentHistoryItem {
            source_session_id: AgentSessionId::new(1),
            item_id: crate::AgentHistoryItemId::new(1),
            item_revision: 0,
            turn_id: Some(AgentTurnId::new(1)),
            logical_segment: 0,
            logical_order: 0,
            turn_working_millis: None,
            content: reasoning(Some(
                hanafubuki_agent_model_api::ModelReasoningReplay::ReasoningContent {
                    text: "Whole thought.".to_string(),
                },
            )),
        }],
        older_cursor: None,
        newer_cursor: None,
        through_revision: AgentRevision::new(1),
    };
    let encoded = serde_json::to_value(page).unwrap();
    assert!(encoded["items"][0]["content"].get("replay").is_none());
    assert_eq!(encoded["items"][0]["content"]["text"], "Shown thought.");
}

fn coloured_output(
    display_metadata: hanafubuki_agent_contracts::AgentToolDisplayMetadata,
) -> ConversationItem {
    ConversationItem::ToolOutput {
        call_id: "call-1".to_string(),
        origin: hanafubuki_agent_model_api::ModelToolCallOrigin::Function,
        output: hanafubuki_agent_tool_contracts::ToolOutput::structured(
            serde_json::json!({ "type": "unified_exec", "output": "red\n" }),
            false,
        ),
        truncation: None,
        images: Vec::new(),
        tool_search_results: Vec::new(),
        display_metadata,
    }
}

fn red_styles() -> hanafubuki_agent_contracts::AgentToolDisplayMetadata {
    hanafubuki_agent_contracts::AgentToolDisplayMetadata {
        styled_output: Some(hanafubuki_agent_contracts::AgentStyledText {
            text_bytes: 4,
            spans: vec![hanafubuki_agent_contracts::AgentStyleSpan {
                start: 0,
                end: 3,
                style: hanafubuki_agent_contracts::AgentTextStyle {
                    fg: Some(hanafubuki_agent_contracts::AgentTextColor::Indexed(1)),
                    ..Default::default()
                },
            }],
        }),
    }
}

/// A tool output's display metadata is persisted beside it, never lowered
/// to the model, absent from older records, and dropped alone when it does
/// not decode.
#[test]
fn a_tool_outputs_display_metadata_is_persisted_tolerantly_and_never_lowered() {
    let styled = coloured_output(red_styles());
    let plain = coloured_output(Default::default());
    assert_eq!(styled.as_model_input(), plain.as_model_input());
    let encoded = serde_json::to_value(&styled).unwrap();
    assert!(encoded.get("displayMetadata").is_some(), "{encoded}");
    assert_eq!(
        serde_json::from_value::<ConversationItem>(encoded.clone()).unwrap(),
        styled
    );
    // Without styles, the record is what it was before display metadata.
    let older = serde_json::to_value(&plain).unwrap();
    assert!(older.get("displayMetadata").is_none(), "{older}");
    assert_eq!(
        serde_json::from_value::<ConversationItem>(older).unwrap(),
        plain
    );
    for broken in [
        serde_json::json!("red"),
        serde_json::json!({ "styledOutput": { "spans": "all" } }),
    ] {
        let mut record = encoded.clone();
        record["displayMetadata"] = broken;
        assert_eq!(
            serde_json::from_value::<ConversationItem>(record).unwrap(),
            plain
        );
    }
    assert!(styled.retained_bytes() > plain.retained_bytes());
}

/// A `!command`'s item is the person's context for the model, and reads
/// back with its optional fields defaulted.
#[test]
fn a_user_shell_command_is_user_context_and_decodes_tolerantly() {
    let item: ConversationItem = serde_json::from_value(serde_json::json!({
        "type": "user_shell_command",
        "callId": "user-shell-3",
        "command": "ls",
        "unknownField": true
    }))
    .unwrap();
    assert_eq!(
        item,
        ConversationItem::UserShellCommand {
            call_id: "user-shell-3".to_string(),
            command: "ls".to_string(),
            cwd: None,
            exit_code: None,
            duration_millis: 0,
            output: String::new(),
            truncated: false,
            styled: None,
        }
    );
    let Some(hanafubuki_agent_model_api::ModelInputItem::ContextFragment {
        role: hanafubuki_agent_model_api::ModelContextRole::User,
        kind,
        content,
    }) = item.as_model_input()
    else {
        panic!("not user context");
    };
    assert_eq!(kind, "user_shell_command");
    assert!(content.starts_with("<user_shell_command>\n<command>\nls\n</command>"));
}
