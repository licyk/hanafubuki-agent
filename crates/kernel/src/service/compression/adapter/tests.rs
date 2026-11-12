use super::*;

fn history_item(item_id: u64, content: ConversationItem) -> Arc<AgentHistoryItem> {
    Arc::new(AgentHistoryItem {
        source_session_id: AgentSessionId::new(1),
        item_id: AgentHistoryItemId::new(item_id),
        item_revision: 0,
        turn_id: None,
        logical_segment: 0,
        logical_order: item_id,
        turn_working_millis: None,
        content,
    })
}

fn reasoning(text: &str) -> ConversationItem {
    ConversationItem::Reasoning {
        text: text.into(),
        duration_millis: 0,
        sampling_step: 1,
        truncated: false,
        replay: hanafubuki_agent_model_api::ModelReasoningReplay::reasoning_content(text.into()),
    }
}

fn assistant(id: u64, text: &str) -> ConversationItem {
    ConversationItem::AssistantMessage {
        message_id: AgentMessageId::new(id),
        phase: None,
        text: text.into(),
        model_text: None,
        proposed_plan: None,
        sampling_step: 1,
        memory_citations: Vec::new(),
    }
}

fn user(text: &str) -> ConversationItem {
    ConversationItem::UserMessage {
        text: text.into(),
        images: Vec::new(),
    }
}

/// A block folds the first answer, and its summary is placed before the
/// second: the first answer's reasoning leaves with it, and the second's
/// stays right before its message, after the summary.
#[test]
fn reasoning_stays_with_the_message_it_preceded() {
    let items = vec![
        history_item(
            1,
            ConversationItem::TurnStart {
                turn_id: AgentTurnId::new(1),
            },
        ),
        history_item(2, user("Look.")),
        history_item(3, reasoning("Folded thought.")),
        history_item(4, assistant(1, "Folded answer.")),
        history_item(5, reasoning("Kept thought.")),
        history_item(6, assistant(2, "Kept answer.")),
    ];
    let log = MessageLog::new(&items);
    let summary = Message::text("acp_summary_1", Role::System, "Summary.");
    let view = vec![log.messages[0].clone(), summary, log.messages[2].clone()];

    let projected = log.apply_view(&view);

    assert!(matches!(projected[0], ConversationItem::TurnStart { .. }));
    assert_eq!(projected[1], user("Look."));
    assert!(matches!(
        &projected[2],
        ConversationItem::CompactionSummary { summary, .. } if summary == "Summary."
    ));
    assert_eq!(projected[3], reasoning("Kept thought."));
    assert_eq!(projected[4], assistant(2, "Kept answer."));
    assert_eq!(projected.len(), 5);
}
