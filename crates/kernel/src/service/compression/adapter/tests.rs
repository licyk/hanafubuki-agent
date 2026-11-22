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
fn compression_keeps_namespaced_calls_and_results_distinct_from_core_tools() {
    let call = ConversationItem::ToolCall {
        call_id: "call-1".into(),
        namespace: Some("remote".into()),
        name: "compress".into(),
        origin: Default::default(),
        catalog_revision: crate::AgentToolCatalogRevision([0; 32]),
        provider_id: crate::AgentToolProviderId::parse("remote").unwrap(),
        tool_id: crate::AgentToolId::parse("compress").unwrap(),
        tool_generation: crate::AgentToolGeneration([0; 32]),
        arguments: serde_json::json!({}),
    };
    let output = ConversationItem::ToolOutput {
        call_id: "call-1".into(),
        origin: Default::default(),
        output: ToolOutput::structured(serde_json::json!({"result":"ok"}), false),
        truncation: None,
        images: Vec::new(),
        tool_search_results: Vec::new(),
        display_metadata: Default::default(),
    };
    let items = vec![
        history_item(1, call.clone()),
        history_item(2, output.clone()),
    ];
    let log = MessageLog::new(&items);
    assert_eq!(log.messages.len(), 2);
    assert!(log
        .messages
        .iter()
        .all(|message| message.tool_name.as_deref() == Some("remote__compress")));
    assert_eq!(log.apply_view(&log.messages), vec![call, output]);
}

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

fn generate_layout(parts: Vec<GeminiReplayPart>) -> ConversationItem {
    use hanafubuki_agent_model_api::{ModelApiProtocol, ModelEndpoint, ModelRouteKey};

    let endpoint = ModelEndpoint::parse("https://google.test/v1beta").unwrap();
    let route = ModelRouteKey::new(
        ModelApiProtocol::GeminiGenerateContent,
        &endpoint,
        "gemini-3-flash",
    );
    ConversationItem::Reasoning {
        text: String::new(),
        duration_millis: 0,
        sampling_step: 1,
        truncated: false,
        replay: Some(ModelReasoningReplay::GeminiGenerate { route, parts }),
    }
}

fn text_part(text: &str) -> GeminiReplayPart {
    GeminiReplayPart::Text {
        text: text.into(),
        signature: Some("signed-text".into()),
        thought: false,
    }
}

fn call_part(id: &str, signature: Option<&str>) -> GeminiReplayPart {
    GeminiReplayPart::Call {
        call_id: id.into(),
        provider_id: Some(format!("wire-{id}")),
        signature: signature.map(str::to_owned),
    }
}

fn call(id: &str) -> ConversationItem {
    ConversationItem::ToolCall {
        call_id: id.into(),
        namespace: None,
        name: "read".into(),
        origin: Default::default(),
        catalog_revision: crate::AgentToolCatalogRevision([0; 32]),
        provider_id: crate::AgentToolProviderId::parse("local").unwrap(),
        tool_id: crate::AgentToolId::parse("read").unwrap(),
        tool_generation: crate::AgentToolGeneration([0; 32]),
        arguments: serde_json::json!({"path":id}),
    }
}

#[test]
fn a_generate_layout_leaves_with_the_response_it_follows() {
    let answer = assistant(1, "Signed response");
    let layout = generate_layout(vec![text_part("Signed response")]);
    let next = user("Next question");
    let items = vec![
        history_item(1, answer.clone()),
        history_item(2, layout.clone()),
        history_item(3, next.clone()),
    ];
    let log = MessageLog::new(&items);
    // Removing the next user message must still retain the signed answer.
    assert_eq!(log.apply_view(&log.messages[..1]), vec![answer, layout]);
    let summary = Message::text("acp_summary_1", Role::System, "Folded.");
    let projected = log.apply_view(&[summary, log.messages[1].clone()]);
    assert!(
        matches!(&projected[..],[ConversationItem::CompactionSummary{summary,..}, kept] if summary=="Folded." && kept==&next)
    );
}

#[test]
fn a_partial_generate_layout_only_signs_the_calls_still_shown_with_their_original_arguments() {
    let hide = assistant(1, "Do not restore the folded text");
    let show = assistant(2, "Original visible text");
    let first = call("first");
    let second = call("second");
    let layout = generate_layout(vec![
        text_part("Do not restore the folded text"),
        text_part("Original visible text"),
        call_part("first", Some("sig-a")),
        call_part("second", Some("sig-b")),
    ]);
    let items = vec![
        history_item(1, hide),
        history_item(2, show),
        history_item(3, first.clone()),
        history_item(4, second.clone()),
        history_item(5, layout),
    ];
    let log = MessageLog::new(&items);
    let mut shown = log.messages[1..].to_vec();
    shown[0].text = "[m2] Clipped visible text".into();
    shown[2].text = r#"{"path":"short"}"#.into();
    let projected = log.apply_view(&shown);
    assert_eq!(projected.len(), 5);
    assert!(
        matches!(&projected[0],ConversationItem::AssistantMessage{model_text:Some(text),..} if text=="[m2] Clipped visible text")
    );
    assert_eq!(projected[1], first);
    assert_eq!(
        projected[2],
        generate_layout(vec![call_part("first", Some("sig-a"))])
    );
    assert!(
        matches!(&projected[3],ConversationItem::ToolCall{arguments,..} if arguments["path"]=="short")
    );
    assert_eq!(
        projected[4],
        generate_layout(vec![call_part("second", None)])
    );

    let kept = log.apply_view(&log.messages[3..]);
    assert_eq!(
        kept,
        vec![
            second,
            generate_layout(vec![call_part("second", Some("sig-b"))])
        ]
    );
}
