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
