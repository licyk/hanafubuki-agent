use super::*;

use hanafubuki_agent_kernel::AgentReasoningId;
use hanafubuki_agent_model_api::ModelReasoningTextKind;

#[tokio::test]
async fn reasoning_effort_is_shared_across_default_and_plan_modes() {
    let model = RecordingAssistantModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut session_config = config();
    session_config.reasoning_effort = Some(hanafubuki_agent_model_api::ModelReasoningEffort::High);
    let session = service.create_session(session_config).unwrap();

    service.start_turn(session.id, "default").await.unwrap();
    let completed = service.snapshot(session.id).unwrap();
    service
        .set_session_mode(
            session.id,
            completed.generation,
            completed.revision,
            hanafubuki_agent_kernel::AgentSessionMode::Plan,
        )
        .unwrap();
    service.start_turn(session.id, "plan").await.unwrap();

    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].reasoning.as_ref().map(|value| &value.effort),
        Some(&hanafubuki_agent_model_api::ModelReasoningEffort::High)
    );
    assert_eq!(
        requests[1].reasoning.as_ref().map(|value| &value.effort),
        Some(&hanafubuki_agent_model_api::ModelReasoningEffort::High)
    );

    let fallback_model = RecordingAssistantModel::default();
    let fallback_service = AgentService::new(
        Arc::new(fallback_model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut fallback_config = config();
    fallback_config.mode = hanafubuki_agent_kernel::AgentSessionMode::Plan;
    fallback_config.reasoning_effort =
        Some(hanafubuki_agent_model_api::ModelReasoningEffort::Medium);
    let fallback_session = fallback_service.create_session(fallback_config).unwrap();
    fallback_service
        .start_turn(fallback_session.id, "fallback")
        .await
        .unwrap();
    assert_eq!(
        fallback_model.requests()[0]
            .reasoning
            .as_ref()
            .map(|value| &value.effort),
        Some(&hanafubuki_agent_model_api::ModelReasoningEffort::Medium)
    );

    let default_model = RecordingAssistantModel::default();
    let default_service = AgentService::new(
        Arc::new(default_model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let default_session = default_service.create_session(config()).unwrap();
    default_service
        .start_turn(default_session.id, "provider default")
        .await
        .unwrap();
    assert!(default_model.requests()[0].reasoning.is_none());
}

#[tokio::test]
async fn reasoning_and_unknown_items_preserve_order_without_becoming_messages_or_tools() {
    let expected = request(vec![ModelInputItem::user_text("finish")]);
    let events = vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "response-extra".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::Reasoning {
                item_id: "reason-1".to_string(),
                summary: Vec::new(),
                content: Vec::new(),
            },
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::Reasoning {
                item_id: "reason-2".to_string(),
                summary: Vec::new(),
                content: Vec::new(),
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Reasoning {
                item_id: "reason-1".to_string(),
                summary: vec!["first summary".to_string()],
                content: Vec::new(),
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Reasoning {
                item_id: "reason-2".to_string(),
                summary: vec!["second summary".to_string()],
                content: Vec::new(),
            },
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::Other {
                item_id: Some("other-1".to_string()),
                item_type: "computer_call".to_string(),
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Other {
                item_id: Some("other-1".to_string()),
                item_type: "computer_call".to_string(),
            },
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id: "message-1".to_string(),
                phase: None,
                text: String::new(),
            },
        }),
        Ok(ModelEvent::AssistantTextDelta {
            item_id: "message-1".to_string(),
            delta: "done".to_string(),
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: "message-1".to_string(),
                phase: None,
                text: "done".to_string(),
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response-extra".to_string(),
            usage: None,
            end_turn: Some(true),
        }),
    ];
    let model = FakeModelTransport::new([FakeModelExchange::new(expected, events)]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service.start_turn(session.id, "finish").await.unwrap();

    assert_eq!(outcome.final_answer, "done");
    let view = test_session_view(&service, session.id).unwrap();
    assert_eq!(
        view.model_context
            .iter()
            .filter(|item| matches!(item, ConversationItem::AssistantMessage { .. }))
            .count(),
        1
    );
    let turn = view.runtime.turn.unwrap();
    assert_eq!(turn.reasoning.sampling_step, 1);
    assert_eq!(
        turn.reasoning.phase,
        hanafubuki_agent_kernel::AgentReasoningPhase::Completed
    );
    assert!(turn.reasoning.active_since.is_none());
    assert!(turn.reasoning.last_completed_at.is_some());
    assert!(turn.reasoning.step_completed_millis <= turn.reasoning.turn_completed_millis);

    let events = service.events_since(session.id, None).unwrap().events;
    let not_observed = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                AgentEventKind::ReasoningUpdated { reasoning }
                    if reasoning.phase == hanafubuki_agent_kernel::AgentReasoningPhase::NotObserved
            )
        })
        .unwrap();
    let thinking = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                AgentEventKind::ReasoningUpdated { reasoning }
                    if reasoning.phase == hanafubuki_agent_kernel::AgentReasoningPhase::Thinking
            )
        })
        .unwrap();
    let completed = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                AgentEventKind::ReasoningUpdated { reasoning }
                    if reasoning.phase == hanafubuki_agent_kernel::AgentReasoningPhase::Completed
            )
        })
        .unwrap();
    let assistant_started = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::AssistantMessageStarted { .. }))
        .unwrap();
    assert!(not_observed < thinking);
    assert!(thinking < completed);
    assert!(completed < assistant_started);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.kind, AgentEventKind::ReasoningUpdated { .. }))
            .count(),
        3,
        "step reset, first item start, and last item completion are the only public updates",
    );

    assert_eq!(
        reasoning_items(&view.model_context)
            .into_iter()
            .map(|item| item.0)
            .collect::<Vec<_>>(),
        ["first summary", "second summary"],
        "overlapping reasoning items each keep their own text, in start order",
    );
}

#[tokio::test]
async fn stream_failure_closes_active_reasoning_before_the_terminal_event() {
    let expected = request(vec![ModelInputItem::user_text("fail")]);
    let events = vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "response-reasoning-failure".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::Reasoning {
                item_id: "reason-active".to_string(),
                summary: vec!["private reasoning".to_string()],
                content: Vec::new(),
            },
        }),
        Err(ModelError::StreamInterrupted(
            "fixture interrupted".to_string(),
        )),
    ];
    let model = FakeModelTransport::new([FakeModelExchange::new(expected, events)]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    assert!(matches!(
        service.start_turn(session.id, "fail").await,
        Err(AgentError::Model(_))
    ));
    let turn = service.snapshot(session.id).unwrap().turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Failed);
    assert_eq!(
        turn.reasoning.phase,
        hanafubuki_agent_kernel::AgentReasoningPhase::Completed
    );
    assert!(turn.reasoning.active_since.is_none());

    let events = service.events_since(session.id, None).unwrap().events;
    let completion = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                AgentEventKind::ReasoningUpdated { reasoning }
                    if reasoning.phase == hanafubuki_agent_kernel::AgentReasoningPhase::Completed
            )
        })
        .unwrap();
    let terminal = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::TurnFailed { .. }))
        .unwrap();
    assert!(completion < terminal);
    assert!(!serde_json::to_string(&(turn, events))
        .unwrap()
        .contains("private reasoning"));
}

/// Prepends a reasoning item, with its streamed fragments and the summary of
/// its completed item, to a deterministic fixture response.
fn with_reasoning(
    item_id: &str,
    fragments: &[(ModelReasoningTextKind, u32, &str)],
    summary: &[&str],
    mut events: Vec<ModelResult<ModelEvent>>,
) -> Vec<ModelResult<ModelEvent>> {
    let mut reasoning = vec![Ok(ModelEvent::OutputItemAdded {
        item: ModelOutputItem::Reasoning {
            item_id: item_id.to_string(),
            summary: Vec::new(),
            content: Vec::new(),
        },
    })];
    for (kind, index, delta) in fragments {
        reasoning.push(Ok(ModelEvent::ReasoningDelta {
            item_id: item_id.to_string(),
            kind: *kind,
            index: *index,
            delta: (*delta).to_string(),
        }));
    }
    reasoning.push(Ok(ModelEvent::OutputItemCompleted {
        item: ModelOutputItem::Reasoning {
            item_id: item_id.to_string(),
            summary: summary.iter().map(|part| (*part).to_string()).collect(),
            content: Vec::new(),
        },
    }));
    assert!(matches!(events[0], Ok(ModelEvent::ResponseStarted { .. })));
    events.splice(1..1, reasoning);
    events
}

/// The reasoning events of a session, as `(id, started step, text, duration)`
/// per reasoning item in the order they started.
fn reasoning_blocks(events: &[AgentEvent]) -> Vec<(AgentReasoningId, u32, String, Option<u64>)> {
    let mut blocks = Vec::<(AgentReasoningId, u32, String, Option<u64>)>::new();
    for event in events {
        match &event.kind {
            AgentEventKind::ReasoningStarted {
                reasoning_id,
                sampling_step,
            } => blocks.push((*reasoning_id, *sampling_step, String::new(), None)),
            AgentEventKind::ReasoningDelta {
                reasoning_id,
                delta,
            } => {
                let block = blocks.iter_mut().find(|block| block.0 == *reasoning_id);
                let block = block.expect("a delta follows its start");
                assert!(block.3.is_none(), "no delta after completion");
                block.2.push_str(delta);
            }
            AgentEventKind::ReasoningCompleted {
                reasoning_id,
                duration_millis,
            } => {
                let block = blocks.iter_mut().find(|block| block.0 == *reasoning_id);
                block.expect("a completion follows its start").3 = Some(*duration_millis);
            }
            _ => {}
        }
    }
    blocks
}

fn reasoning_items(items: &[ConversationItem]) -> Vec<(String, u64, u32, bool)> {
    items
        .iter()
        .filter_map(|item| match item {
            ConversationItem::Reasoning {
                text,
                duration_millis,
                sampling_step,
                truncated,
            } => Some((text.clone(), *duration_millis, *sampling_step, *truncated)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn streamed_reasoning_reaches_events_and_history_but_never_model_input() {
    let tool_output =
        hanafubuki_agent_tool_contracts::ToolOutput::structured(json!({ "instances": [] }), false);
    let model = ScriptedEventModel::new([
        with_reasoning(
            "reason-1",
            &[
                (ModelReasoningTextKind::Summary, 0, "**Mapping"),
                (ModelReasoningTextKind::Summary, 0, " the app**"),
                (ModelReasoningTextKind::Summary, 1, "Checking"),
                (ModelReasoningTextKind::Summary, 1, " tests"),
            ],
            &["**Mapping the app**", "Checking tests"],
            deterministic_tool_call_events(),
        ),
        with_reasoning("reason-2", &[], &[], deterministic_assistant_events("done")),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        tool_output,
    )])
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service.start_turn(session.id, "finish").await.unwrap();
    assert_eq!(outcome.final_answer, "done");

    let events = service.events_since(session.id, None).unwrap().events;
    let blocks = reasoning_blocks(&events);
    assert_eq!(blocks.len(), 2);
    assert_ne!(blocks[0].0, blocks[1].0);
    assert_eq!(
        (blocks[0].1, blocks[0].2.as_str()),
        (1, "**Mapping the app**\n\nChecking tests"),
        "summary parts are joined by a blank line",
    );
    assert_eq!((blocks[1].1, blocks[1].2.as_str()), (2, ""));
    let first_completed = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::ReasoningCompleted { .. }))
        .unwrap();
    let first_commit = events
        .iter()
        .enumerate()
        .skip(first_completed)
        .find(|(_, event)| matches!(event.kind, AgentEventKind::HistoryCommitted { .. }))
        .map(|(index, _)| index)
        .unwrap();
    let tool_started = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::ToolStarted { .. }))
        .unwrap();
    assert!(first_completed < first_commit && first_commit < tool_started);

    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert_eq!(
        reasoning_items(&history),
        vec![
            (
                "**Mapping the app**\n\nChecking tests".to_string(),
                blocks[0].3.unwrap(),
                1,
                false
            ),
            (String::new(), blocks[1].3.unwrap(), 2, false),
        ],
        "each item keeps the shown text and the duration its completion carried",
    );
    let reasoning = history
        .iter()
        .position(|item| matches!(item, ConversationItem::Reasoning { .. }))
        .unwrap();
    assert!(matches!(
        history[reasoning + 1],
        ConversationItem::ToolCall { .. }
    ));
    let answer = history
        .iter()
        .position(|item| matches!(item, ConversationItem::AssistantMessage { .. }))
        .unwrap();
    assert!(matches!(
        history[answer - 1],
        ConversationItem::Reasoning {
            sampling_step: 2,
            ..
        }
    ));

    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    let second_input = serde_json::to_string(&requests[1].input).unwrap();
    assert!(second_input.contains(FIXTURE_CALL_ID));
    assert!(
        !second_input.contains("Mapping the app"),
        "thinking is never sent back to the model"
    );
}

#[tokio::test]
async fn a_summary_that_arrives_whole_is_shown_at_completion_and_the_summary_outranks_raw_text() {
    let model = ScriptedEventModel::new([with_reasoning(
        "reason-1",
        &[(ModelReasoningTextKind::Raw, 0, "raw thinking")],
        &["The summary."],
        with_reasoning("reason-0", &[], &["Whole summary."], {
            deterministic_assistant_events("done")
        }),
    )]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    service.start_turn(session.id, "finish").await.unwrap();

    let events = service.events_since(session.id, None).unwrap().events;
    let shown = reasoning_blocks(&events)
        .into_iter()
        .map(|block| block.2)
        .collect::<Vec<_>>();
    assert_eq!(shown, ["raw thinking", "Whole summary."]);
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    let kept = reasoning_items(&history)
        .into_iter()
        .map(|item| item.0)
        .collect::<Vec<_>>();
    assert_eq!(
        kept,
        ["The summary.", "Whole summary."],
        "an item that has a summary keeps it, even after streaming raw text",
    );
}

#[tokio::test]
async fn long_thinking_is_cut_at_the_reasoning_bound_and_spends_no_output_budget() {
    let limits = AgentHardLimits {
        maximum_model_output_bytes: 64,
        maximum_reasoning_text_bytes: 10,
        ..FIRST_BATCH_HARD_LIMITS
    };
    let fragments = (0..40)
        .map(|_| (ModelReasoningTextKind::Raw, 0, "思考"))
        .collect::<Vec<_>>();
    let model = ScriptedEventModel::new([with_reasoning(
        "reason-1",
        &fragments,
        &[],
        with_reasoning(
            "reason-2",
            &[],
            &["a summary that arrives whole"],
            deterministic_assistant_events("done"),
        ),
    )]);
    let service = AgentService::with_limits(
        Arc::new(model),
        FakeTools::new([]).await.registries(),
        limits,
    );
    let session = service.create_session(config()).unwrap();

    let outcome = service.start_turn(session.id, "finish").await.unwrap();

    assert_eq!(
        outcome.final_answer, "done",
        "240 bytes of thinking do not fail a turn whose output bound is 64"
    );
    let events = service.events_since(session.id, None).unwrap().events;
    let blocks = reasoning_blocks(&events);
    assert_eq!(blocks[0].2, "思考思", "cut on a character boundary");
    assert!(blocks[0].3.is_some());
    assert_eq!(blocks[1].2, "a summary ");
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert_eq!(
        reasoning_items(&history)
            .into_iter()
            .map(|item| (item.0, item.3))
            .collect::<Vec<_>>(),
        [
            ("思考思".to_string(), true),
            ("a summary ".to_string(), true)
        ]
    );
}

#[tokio::test]
async fn an_incomplete_response_completes_its_open_reasoning_and_keeps_what_streamed() {
    let model = ScriptedEventModel::new([vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "partial".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::Reasoning {
                item_id: "thinking".to_string(),
                summary: Vec::new(),
                content: Vec::new(),
            },
        }),
        Ok(ModelEvent::ReasoningDelta {
            item_id: "thinking".to_string(),
            kind: ModelReasoningTextKind::Raw,
            index: 0,
            delta: "Half a thought".to_string(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id: "answer".to_string(),
                phase: None,
                text: String::new(),
            },
        }),
        Ok(ModelEvent::AssistantTextDelta {
            item_id: "answer".to_string(),
            delta: "The partial ans".to_string(),
        }),
        Ok(ModelEvent::ResponseIncomplete {
            response_id: "partial".to_string(),
            usage: None,
            reason: "max_output_tokens".to_string(),
        }),
    ]]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service.start_turn(session.id, "Explain").await.unwrap();

    assert_eq!(outcome.final_answer, "The partial ans");
    let events = service.events_since(session.id, None).unwrap().events;
    let blocks = reasoning_blocks(&events);
    assert_eq!(blocks[0].2, "Half a thought");
    assert!(blocks[0].3.is_some(), "the open item is completed");
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert_eq!(reasoning_items(&history)[0].0, "Half a thought");
}

#[tokio::test]
async fn a_response_of_only_reasoning_still_commits_nothing() {
    let mut events = with_reasoning(
        "reason-1",
        &[(ModelReasoningTextKind::Raw, 0, "thinking")],
        &[],
        vec![Ok(ModelEvent::ResponseStarted {
            response_id: "thinking-only".to_string(),
            metadata: ModelResponseMetadata::default(),
        })],
    );
    events.push(Ok(ModelEvent::ResponseCompleted {
        response_id: "thinking-only".to_string(),
        usage: None,
        end_turn: Some(true),
    }));
    let model = ScriptedEventModel::new([events]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    let result = service.start_turn(session.id, "finish").await;

    assert!(
        matches!(&result, Err(AgentError::Protocol(message)) if message.contains("no conversation item")),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_reasoning_delta_for_an_unknown_reasoning_item_is_a_protocol_error() {
    let model = ScriptedEventModel::new([vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "response-orphan".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::ReasoningDelta {
            item_id: "reason-missing".to_string(),
            kind: ModelReasoningTextKind::Summary,
            index: 0,
            delta: "orphan".to_string(),
        }),
    ]]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    let result = service.start_turn(session.id, "finish").await;

    assert!(matches!(
        &result,
        Err(AgentError::Protocol(message))
            if message.contains("unknown or completed reasoning item")
    ));
    let events = service.events_since(session.id, None).unwrap().events;
    assert!(reasoning_blocks(&events).is_empty());
}
