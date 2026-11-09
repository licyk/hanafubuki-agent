use hanafubuki_agent::{AgentRevision, AgentTurnId, ConversationItem};

use super::PromptScan;

fn turn(id: u64) -> ConversationItem {
    ConversationItem::TurnStart {
        turn_id: AgentTurnId::new(id),
    }
}

fn user(text: &str) -> ConversationItem {
    ConversationItem::UserMessage {
        text: text.to_string(),
        images: Vec::new(),
    }
}

fn steering(text: &str) -> ConversationItem {
    serde_json::from_value(serde_json::json!({
        "type": "steering_message",
        "sequence": 1,
        "text": text,
    }))
    .unwrap()
}

/// A note the kernel adds for the model (a side policy, an approved
/// action): never a prompt.
fn continuation(content: &str) -> ConversationItem {
    serde_json::from_value(serde_json::json!({
        "type": "continuation_context",
        "providerId": "kernel",
        "hookId": "approved-denied-action",
        "generation": vec![0; 32],
        "content": content,
    }))
    .unwrap()
}

fn compaction() -> ConversationItem {
    ConversationItem::CompactionSummary {
        summary: "Earlier work.".to_string(),
        source_revision: AgentRevision::new(1),
        compacted_item_count: 3,
    }
}

/// The turn of the last of `prompts` in `items`, given oldest first as a
/// session holds them.
fn scan(items: &[ConversationItem], prompts: &[&str]) -> Result<Option<AgentTurnId>, String> {
    let prompts: Vec<String> = prompts.iter().map(|text| text.to_string()).collect();
    let mut scan = PromptScan::new(&prompts);
    for item in items.iter().rev() {
        if let Some(turn_id) = scan.push(item)? {
            return Ok(Some(turn_id));
        }
    }
    Ok(None)
}

#[test]
fn a_prompt_counted_from_the_latest_maps_to_its_turn_past_the_items_that_are_not_prompts() {
    let items = [
        turn(1),
        user("First."),
        compaction(),
        turn(2),
        continuation("The person approved the action."),
        user("Second."),
        continuation("Side conversation boundary."),
        turn(3),
        user("Third."),
    ];
    assert_eq!(scan(&items, &["Third."]), Ok(Some(AgentTurnId::new(3))));
    assert_eq!(
        scan(&items, &["Third.", "Second."]),
        Ok(Some(AgentTurnId::new(2)))
    );
    assert_eq!(
        scan(&items, &["Third.", "Second.", "First."]),
        Ok(Some(AgentTurnId::new(1)))
    );
    assert_eq!(
        scan(&items, &["Third.", "Second.", "First.", "Zeroth."]),
        Ok(None)
    );
}

#[test]
fn a_steering_message_is_a_prompt_but_not_a_turn_to_edit() {
    let items = [
        turn(1),
        user("Start."),
        steering("Also this."),
        turn(2),
        user("Next."),
    ];
    assert_eq!(
        scan(&items, &["Next.", "Also this.", "Start."]),
        Ok(Some(AgentTurnId::new(1)))
    );
    let error = scan(&items, &["Next.", "Also this."]).unwrap_err();
    assert!(error.contains("cannot be edited on its own"), "{error}");
}

#[test]
fn a_shown_prompt_the_session_does_not_hold_fails_the_mapping() {
    let items = [turn(1), user("First."), turn(2), user("Second.")];
    // The latest prompt shown never reached the session.
    let error = scan(&items, &["Refused.", "Second."]).unwrap_err();
    assert!(error.contains("no longer matches"), "{error}");
}
