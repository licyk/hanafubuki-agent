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
