use super::*;

#[tokio::test]
async fn completed_assistant_message_is_committed_before_terminal_success() {
    let expected = request(vec![ModelInputItem::user_text("hello")]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("world"),
    )]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    service
        .start_turn(session.id, "hello".to_string())
        .await
        .unwrap();

    let completed = test_session_view(&service, session.id).unwrap();
    let final_answer_message_id = completed
        .turn
        .as_ref()
        .unwrap()
        .final_answer_message_id
        .unwrap();
    assert_eq!(
        completed.model_context,
        vec![
            ConversationItem::TurnStart {
                turn_id: test_session_view(&service, session.id)
                    .unwrap()
                    .runtime
                    .turn
                    .unwrap()
                    .id,
            },
            ConversationItem::UserMessage {
                text: "hello".to_string(),
                images: Vec::new(),
            },
            ConversationItem::AssistantMessage {
                message_id: final_answer_message_id,
                phase: Some(hanafubuki_agent_kernel::AgentMessagePhase::FinalAnswer),
                text: "world".to_string(),
                model_text: None,
                proposed_plan: None,
                sampling_step: 1,
                memory_citations: Vec::new(),
            },
        ]
    );
}

#[tokio::test]
async fn oversized_model_output_fails_without_committing_partial_assistant_history() {
    let limits = AgentHardLimits {
        maximum_model_output_bytes: 3,
        ..FIRST_BATCH_HARD_LIMITS
    };
    let expected = request(vec![ModelInputItem::user_text("hello")]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("world"),
    )]);
    let service = AgentService::with_limits(
        Arc::new(model),
        FakeTools::new([]).await.registries(),
        limits,
    );
    let session = service.create_session(config()).unwrap();

    assert_eq!(
        service.start_turn(session.id, "hello".to_string()).await,
        Err(AgentError::LimitExceeded {
            resource: "model output bytes",
            limit: 3,
        })
    );
    let snapshot = test_session_view(&service, session.id).unwrap();
    assert_eq!(
        snapshot.model_context,
        vec![
            ConversationItem::TurnStart {
                turn_id: snapshot.turn.as_ref().unwrap().id,
            },
            ConversationItem::UserMessage {
                text: "hello".to_string(),
                images: Vec::new(),
            },
        ]
    );
    assert_eq!(snapshot.runtime.turn.unwrap().phase, AgentTurnPhase::Failed);
}

#[tokio::test]
async fn multiple_commentary_tool_batches_commit_distinct_messages_before_final_answer() {
    let model = ScriptedEventModel::new([
        commentary_tool_response(
            "response-one",
            "I will inspect the available instances first.",
            &[("tool-item-a", "call-a"), ("tool-item-b", "call-b")],
        ),
        commentary_tool_response(
            "response-two",
            "The first checks agree; I will verify once more.",
            &[("tool-item-c", "call-c")],
        ),
        deterministic_assistant_events("All checks are complete."),
    ]);
    let tools = FakeTools::new([
        FakeRegisteredExchange::success("list_instances", json!({}), json!({"batch": "a"})),
        FakeRegisteredExchange::success("list_instances", json!({}), json!({"batch": "b"})),
        FakeRegisteredExchange::success("list_instances", json!({}), json!({"batch": "c"})),
    ])
    .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "inspect everything".to_string())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "All checks are complete.");

    let snapshot = test_session_view(&service, session.id).unwrap();
    let messages = snapshot
        .model_context
        .iter()
        .filter_map(|item| match item {
            ConversationItem::AssistantMessage {
                message_id,
                phase,
                text,
                sampling_step,
                ..
            } => Some((*message_id, *phase, text.as_str(), *sampling_step)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        messages
            .iter()
            .map(|(_, phase, text, step)| (*phase, *text, *step))
            .collect::<Vec<_>>(),
        vec![
            (
                Some(hanafubuki_agent_kernel::AgentMessagePhase::Commentary),
                "I will inspect the available instances first.",
                1,
            ),
            (
                Some(hanafubuki_agent_kernel::AgentMessagePhase::Commentary),
                "The first checks agree; I will verify once more.",
                2,
            ),
            (
                Some(hanafubuki_agent_kernel::AgentMessagePhase::FinalAnswer),
                "All checks are complete.",
                3,
            ),
        ]
    );
    let unique_ids = messages
        .iter()
        .map(|(message_id, ..)| *message_id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique_ids.len(), messages.len());
    assert_eq!(
        snapshot.turn.as_ref().unwrap().final_answer_message_id,
        Some(messages[2].0)
    );
    assert_eq!(outcome.final_answer_message_id, messages[2].0);

    let events = service.events_since(session.id, None).unwrap().events;
    for (message_id, call_id) in [(messages[0].0, "call-a"), (messages[1].0, "call-c")] {
        let completed = events
            .iter()
            .position(|event| {
                matches!(
                    event.kind,
                    AgentEventKind::AssistantMessageCompleted {
                        message_id: completed_id,
                        phase: Some(hanafubuki_agent_kernel::AgentMessagePhase::Commentary),
                    } if completed_id == message_id
                )
            })
            .unwrap();
        let tool_started = events
            .iter()
            .position(|event| {
                matches!(
                    &event.kind,
                    AgentEventKind::ToolStarted {
                        call_id: started_call_id,
                        ..
                    } if started_call_id == call_id
                )
            })
            .unwrap();
        assert!(completed < tool_started);
    }
}

#[tokio::test]
async fn missing_phase_is_preserved_while_provider_end_turn_controls_follow_up() {
    let model = ScriptedEventModel::new([
        text_response("continue", "I have one more check.", None, Some(false)),
        text_response("finish", "The check is complete.", None, Some(true)),
    ]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "check".to_string())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "The check is complete.");
    let phases = test_session_view(&service, session.id)
        .unwrap()
        .model_context
        .into_iter()
        .filter_map(|item| match item {
            ConversationItem::AssistantMessage { phase, .. } => Some(phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(phases, vec![None, None]);
}

#[tokio::test]
async fn assistant_message_with_a_tool_follows_up_without_a_private_phase_gate() {
    let mut events = text_response(
        "invalid",
        "This is final.",
        Some(ModelMessagePhase::FinalAnswer),
        Some(false),
    );
    events.pop();
    events.extend([
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "invalid-tool".to_string(),
                    call_id: "invalid-call".to_string(),
                    namespace: None,
                    name: "list_instances".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "invalid-tool".to_string(),
                    call_id: "invalid-call".to_string(),
                    namespace: None,
                    name: "list_instances".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "invalid".to_string(),
            usage: None,
            end_turn: Some(false),
        }),
    ]);
    let service = AgentService::new(
        Arc::new(ScriptedEventModel::new([
            events,
            deterministic_assistant_events("Actually complete.")
                .into_iter()
                .collect(),
        ])),
        FakeTools::new([FakeRegisteredExchange::success(
            "list_instances",
            json!({}),
            json!([]),
        )])
        .await
        .registries(),
    );
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "inspect".to_string())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Actually complete.");
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert!(history.iter().any(|item| matches!(
        item,
        ConversationItem::ToolCall { name, .. } if name == "list_instances"
    )));
}

/// A model may open an output item as commentary and, once it has written the
/// message, complete it as the final answer. That is the model refining its own
/// annotation, and the completed item is authoritative — finalization already
/// prefers it. Treating the change as a protocol violation aborted the whole
/// turn and discarded a finished answer.
#[tokio::test]
async fn a_refined_assistant_phase_keeps_the_completed_item_as_authoritative() {
    let mut events = text_response(
        "refined",
        "The check is complete.",
        Some(ModelMessagePhase::Commentary),
        Some(true),
    );
    // Complete the same item as the final answer instead of commentary.
    events[3] = Ok(ModelEvent::OutputItemCompleted {
        item: ModelOutputItem::AssistantMessage {
            item_id: "refined-message".to_string(),
            phase: Some(ModelMessagePhase::FinalAnswer),
            text: "The check is complete.".to_string(),
        },
    });
    let model = ScriptedEventModel::new([events]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "check".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "The check is complete.");
    let phases = test_session_view(&service, session.id)
        .unwrap()
        .model_context
        .into_iter()
        .filter_map(|item| match item {
            ConversationItem::AssistantMessage { phase, .. } => Some(phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec![Some(
            hanafubuki_agent_kernel::AgentMessagePhase::FinalAnswer
        )]
    );

    // The shell is told the corrected phase, so a message announced as
    // commentary does not stay labelled that way.
    let events = service.events_since(session.id, None).unwrap().events;
    assert!(events.iter().any(|event| matches!(
        event.kind,
        AgentEventKind::AssistantMessageCompleted {
            phase: Some(hanafubuki_agent_kernel::AgentMessagePhase::FinalAnswer),
            ..
        }
    )));
}
