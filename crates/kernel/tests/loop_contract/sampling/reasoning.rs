use super::*;

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
            },
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::Reasoning {
                item_id: "reason-2".to_string(),
                summary: Vec::new(),
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Reasoning {
                item_id: "reason-1".to_string(),
                summary: vec!["not presented".to_string()],
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Reasoning {
                item_id: "reason-2".to_string(),
                summary: vec!["also private".to_string()],
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

    let presentation_json = serde_json::to_string(&(turn, events)).unwrap();
    assert!(!presentation_json.contains("not presented"));
    assert!(!presentation_json.contains("also private"));
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

/// Prepends a reasoning item, with its streamed summary fragments and final
/// summary, to a deterministic fixture response.
fn with_reasoning_summary(
    item_id: &str,
    fragments: &[(u32, &str)],
    summary: &[&str],
    mut events: Vec<ModelResult<ModelEvent>>,
) -> Vec<ModelResult<ModelEvent>> {
    let mut reasoning = vec![Ok(ModelEvent::OutputItemAdded {
        item: ModelOutputItem::Reasoning {
            item_id: item_id.to_string(),
            summary: Vec::new(),
        },
    })];
    for (summary_index, delta) in fragments {
        reasoning.push(Ok(ModelEvent::ReasoningSummaryDelta {
            item_id: item_id.to_string(),
            summary_index: *summary_index,
            delta: (*delta).to_string(),
        }));
    }
    reasoning.push(Ok(ModelEvent::OutputItemCompleted {
        item: ModelOutputItem::Reasoning {
            item_id: item_id.to_string(),
            summary: summary.iter().map(|part| (*part).to_string()).collect(),
        },
    }));
    assert!(matches!(events[0], Ok(ModelEvent::ResponseStarted { .. })));
    events.splice(1..1, reasoning);
    events
}

fn headlines(events: &[AgentEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            AgentEventKind::ReasoningHeadlineUpdated { headline } => Some(headline.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn streamed_reasoning_summaries_publish_a_bounded_headline_that_outlives_tool_steps() {
    let tool_output =
        hanafubuki_agent_tool_contracts::ToolOutput::structured(json!({ "instances": [] }), false);
    let model = ScriptedEventModel::new([
        with_reasoning_summary(
            "reason-1",
            &[
                (0, "**Mapping"),
                (0, " the app**\n\nLooking at routes."),
                (1, "<!-- private note -->"),
                (1, "\n## Checking"),
                (1, " tests"),
            ],
            &[
                "**Mapping the app**\n\nLooking at routes.",
                "<!-- private note -->\n## Checking tests",
            ],
            deterministic_tool_call_events(),
        ),
        with_reasoning_summary("reason-2", &[], &[], deterministic_assistant_events("done")),
        deterministic_assistant_events("second turn"),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        tool_output,
    )])
    .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service.start_turn(session.id, "finish").await.unwrap();
    assert_eq!(outcome.final_answer, "done");

    let events = service.events_since(session.id, None).unwrap().events;
    assert_eq!(
        headlines(&events),
        ["Looking at routes.", "Checking", "Checking tests"],
        "each published headline is the latest presentable summary line, streaming as it grows",
    );
    let turn = service.snapshot(session.id).unwrap().turn.unwrap();
    assert_eq!(turn.sampling_step, 2);
    assert_eq!(turn.reasoning_headline.as_deref(), Some("Checking tests"));
    let last_headline = events
        .iter()
        .rposition(|event| matches!(event.kind, AgentEventKind::ReasoningHeadlineUpdated { .. }))
        .unwrap();
    let second_step = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::SamplingStarted { step: 2 }))
        .unwrap();
    assert!(
        last_headline < second_step,
        "the second step's empty reasoning item publishes nothing and keeps the headline",
    );
    let presentation_json = serde_json::to_string(&(&turn, &events)).unwrap();
    assert!(!presentation_json.contains("private note"));
    assert!(!presentation_json.contains("**Mapping"));

    service.start_turn(session.id, "again").await.unwrap();
    let turn = service.snapshot(session.id).unwrap().turn.unwrap();
    assert_eq!(turn.reasoning_headline, None);
    assert_eq!(
        headlines(&service.events_since(session.id, None).unwrap().events).len(),
        3,
        "a turn without reasoning summaries publishes no headline",
    );
}

#[tokio::test]
async fn a_summary_delta_for_an_unknown_reasoning_item_is_a_protocol_error() {
    let model = ScriptedEventModel::new([vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "response-orphan".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::ReasoningSummaryDelta {
            item_id: "reason-missing".to_string(),
            summary_index: 0,
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
    assert!(headlines(&events).is_empty());
}
