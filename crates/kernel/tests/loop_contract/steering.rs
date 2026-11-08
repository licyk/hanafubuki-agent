use super::*;

#[tokio::test]
async fn steering_during_approval_is_exact_persisted_and_consumed_in_sequence() {
    let first = request(vec![ModelInputItem::user_text("start it")]);
    let tool_output = hanafubuki_agent_tool_contracts::ToolOutput::structured(
        json!({ "operationId": "launch-1", "instanceId": "instance-a" }),
        false,
    );
    let second = request(vec![
        ModelInputItem::user_text("start it"),
        ModelInputItem::ToolCall {
            call_id: "call-mutation".to_string(),
            namespace: None,
            name: "control_instance".to_string(),
            arguments: json!({ "instanceId": "instance-a", "action": "start" }),
        },
        ModelInputItem::ToolOutput {
            call_id: "call-mutation".to_string(),
            output: tool_output.clone(),
        },
        host_context_fragment(r#"{"page":"instance chooser"}"#),
        ModelInputItem::user_text("also report readiness"),
        ModelInputItem::user_text("then stop"),
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first, mutation_tool_call_events()),
        FakeModelExchange::new(second, deterministic_assistant_events("started and ready")),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "control_instance",
        json!({ "instanceId": "instance-a", "action": "start" }),
        tool_output,
    )])
    .await;
    let _host = tools.mount(&host_context_renderer()).await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    let session_id = session.id;
    let executing = service.clone();
    let turn_task = tokio::spawn(async move {
        executing
            .start_turn(session_id, "start it".to_string())
            .await
    });
    let (turn, approval) = loop {
        let snapshot = test_session_view(&service, session_id).unwrap();
        if let Some(turn) = snapshot.runtime.turn {
            if let Some(request) = service
                .events_since(session_id, None)
                .unwrap()
                .events
                .into_iter()
                .find_map(|event| match event.kind {
                    AgentEventKind::ApprovalRequested { request } => Some(request),
                    _ => None,
                })
            {
                break (turn, request);
            }
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    assert_eq!(
        service.steer_turn_with_sink(
            session_id,
            turn.id,
            session.generation,
            "stale".to_string(),
            &NoopEventSink,
        ),
        Err(AgentError::StaleGeneration)
    );
    let first_steering = service
        .steer_turn_with_sink(
            session_id,
            turn.id,
            turn.generation,
            AgentSteeringSubmission {
                text: "also report readiness".to_string(),
                images: Vec::new(),
                host_context: Some(AgentHostContext {
                    kind: HOST_LOCATION,
                    content: r#"{"page":"instance chooser"}"#.to_string(),
                }),
            },
            &NoopEventSink,
        )
        .unwrap();
    let second_steering = service
        .steer_turn_with_sink(
            session_id,
            turn.id,
            turn.generation,
            "then stop".to_string(),
            &NoopEventSink,
        )
        .unwrap();
    assert_eq!(first_steering.sequence.value(), 1);
    assert_eq!(second_steering.sequence.value(), 2);
    assert_eq!(
        service
            .snapshot(session_id)
            .unwrap()
            .turn
            .unwrap()
            .pending_steering_count,
        2
    );
    service
        .resolve_approval(
            session_id,
            turn.id,
            turn.generation,
            approval.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    assert_eq!(
        turn_task.await.unwrap().unwrap().final_answer,
        "started and ready"
    );
    let snapshot = test_session_view(&service, session_id).unwrap();
    let first_steering_index = snapshot
        .model_context
        .iter()
        .position(|item| {
            matches!(
                item,
                ConversationItem::SteeringMessage { sequence, .. }
                    if sequence.value() == 1
            )
        })
        .unwrap();
    assert!(matches!(
        snapshot.model_context[first_steering_index - 1],
        ConversationItem::WorldStateUpdate { .. }
    ));
    let turn = snapshot.runtime.turn.as_ref().unwrap();
    assert_eq!(turn.pending_steering_count, 0);
    assert_eq!(turn.last_consumed_steering_sequence.unwrap().value(), 2);
    assert!(matches!(
        service.steer_turn_with_sink(
            session_id,
            turn.id,
            turn.generation,
            "late".to_string(),
            &NoopEventSink,
        ),
        Err(AgentError::TurnNotRunning)
    ));
}

#[tokio::test]
async fn steering_during_sampling_is_bounded_and_cancellation_closes_admission() {
    let service = AgentService::new(
        Arc::new(CancellationOnlyModel),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service
        .set_session_turn_budget_policy(
            session.id,
            AgentTurnBudgetPolicy {
                maximum_pending_input_items: 2,
                ..AgentTurnBudgetPolicy::default()
            },
        )
        .unwrap();
    let executing = service.clone();
    let task =
        tokio::spawn(async move { executing.start_turn(session.id, "wait".to_string()).await });
    let turn = wait_for_active_turn(&service, session.id).await;

    let first = service
        .steer_turn_with_sink(
            session.id,
            turn.id,
            turn.generation,
            "first".to_string(),
            &NoopEventSink,
        )
        .unwrap();
    let second = service
        .steer_turn_with_sink(
            session.id,
            turn.id,
            turn.generation,
            "second".to_string(),
            &NoopEventSink,
        )
        .unwrap();
    assert_eq!(first.sequence.value(), 1);
    assert_eq!(second.sequence.value(), 2);
    assert_eq!(
        service.steer_turn_with_sink(
            session.id,
            turn.id,
            turn.generation,
            "overflow".to_string(),
            &NoopEventSink,
        ),
        Err(AgentError::LimitExceeded {
            resource: "pending input items",
            limit: 2,
        })
    );

    service
        .request_turn_cancellation(session.id, turn.id, turn.generation)
        .await
        .unwrap();
    assert_eq!(task.await.unwrap(), Err(AgentError::Cancelled));
    assert_eq!(
        service
            .snapshot(session.id)
            .unwrap()
            .turn
            .unwrap()
            .pending_steering_count,
        0
    );
    assert_eq!(
        service.steer_turn_with_sink(
            session.id,
            turn.id,
            turn.generation,
            "late".to_string(),
            &NoopEventSink,
        ),
        Err(AgentError::TurnNotRunning)
    );
}

#[tokio::test]
async fn steering_during_tool_execution_commits_after_the_ordered_tool_output() {
    let first_request = request(vec![ModelInputItem::user_text("inspect")]);
    let tool_output =
        hanafubuki_agent_tool_contracts::ToolOutput::structured(json!({ "instances": [] }), false);
    let second_request = request(vec![
        ModelInputItem::user_text("inspect"),
        ModelInputItem::ToolCall {
            call_id: FIXTURE_CALL_ID.to_string(),
            namespace: None,
            name: "list_instances".to_string(),
            arguments: json!({}),
        },
        ModelInputItem::ToolOutput {
            call_id: FIXTURE_CALL_ID.to_string(),
            output: tool_output.clone(),
        },
        ModelInputItem::user_text("include health"),
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first_request, deterministic_tool_call_events()),
        FakeModelExchange::new(second_request, deterministic_assistant_events("healthy")),
    ]);
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        tool_output,
    )])
    .await
    .with_invoke_gate(entered.clone(), release.clone());
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    let executing = service.clone();
    let task = tokio::spawn(async move {
        executing
            .start_turn(session.id, "inspect".to_string())
            .await
    });
    entered.wait().await;
    let turn = test_session_view(&service, session.id)
        .unwrap()
        .runtime
        .turn
        .unwrap();
    service
        .steer_turn_with_sink(
            session.id,
            turn.id,
            turn.generation,
            "include health".to_string(),
            &NoopEventSink,
        )
        .unwrap();
    release.wait().await;

    assert_eq!(task.await.unwrap().unwrap().final_answer, "healthy");
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    let output_index = history
        .iter()
        .position(|item| matches!(item, ConversationItem::ToolOutput { .. }))
        .unwrap();
    let steering_index = history
        .iter()
        .position(|item| matches!(item, ConversationItem::SteeringMessage { .. }))
        .unwrap();
    assert!(output_index < steering_index);
}

#[tokio::test]
async fn steering_at_the_completion_boundary_prevents_lost_finalization() {
    let provider_id = AgentContinuationHookProviderId::parse("test-provider".to_string()).unwrap();
    let hook_id = AgentContinuationHookId::parse("finalization-gate".to_string()).unwrap();
    let generation = AgentContinuationHookGeneration([9; 32]);
    let expected_second_input = vec![
        ModelInputItem::user_text("draft"),
        ModelInputItem::AssistantMessage {
            phase: Some(ModelMessagePhase::FinalAnswer),
            text: "initial".to_string(),
        },
        ModelInputItem::user_text("late guidance"),
    ];
    let model = RecordingAssistantModel::default();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let hook = Arc::new(BlockingOnceHook {
        blocked: AtomicBool::new(false),
        entered: entered.clone(),
        release: release.clone(),
    });
    let tools = FakeTools::new([]).await;
    tools
        .mount(&registering::<ContinuationHookRegistry>(
            "finalization-gate",
            vec![AgentContinuationHookRegistration {
                provider_id,
                hook_id,
                generation,
                priority: 1,
                failure_policy: AgentContinuationHookFailurePolicy::FailTurn,
                hook,
            }],
        ))
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();
    let executing = service.clone();
    let task =
        tokio::spawn(async move { executing.start_turn(session.id, "draft".to_string()).await });
    entered.wait().await;
    let turn = test_session_view(&service, session.id)
        .unwrap()
        .runtime
        .turn
        .unwrap();
    service
        .steer_turn_with_sink(
            session.id,
            turn.id,
            turn.generation,
            "late guidance".to_string(),
            &NoopEventSink,
        )
        .unwrap();
    release.wait().await;

    assert_eq!(task.await.unwrap().unwrap().final_answer, "final");
    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].input, expected_second_input);
    assert_eq!(
        service
            .snapshot(session.id)
            .unwrap()
            .turn
            .unwrap()
            .last_consumed_steering_sequence
            .unwrap()
            .value(),
        1
    );
}
