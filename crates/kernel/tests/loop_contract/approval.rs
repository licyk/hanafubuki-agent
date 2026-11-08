use super::*;

#[tokio::test]
async fn mutation_requires_exact_one_time_approval_before_tool_dispatch() {
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
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first, mutation_tool_call_events()),
        FakeModelExchange::new(second, deterministic_assistant_events("started")),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "control_instance",
        json!({ "instanceId": "instance-a", "action": "start" }),
        tool_output,
    )])
    .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    let session_id = session.id;
    let executing = service.clone();
    let turn_task = tokio::spawn(async move {
        executing
            .start_turn(session_id, "start it".to_string())
            .await
    });

    let (turn, request) = loop {
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
    assert!(tools.calls().is_empty());
    assert_eq!(request.operation_id, "call-mutation");
    assert_eq!(
        request.action.kind,
        hanafubuki_agent_kernel::ApprovalActionKind::DomainMutation
    );
    assert_eq!(request.action.summary, "Start instance instance-a");
    assert_eq!(
        service.resolve_approval(
            session_id,
            turn.id,
            session.generation,
            request.request_id,
            ApprovalUserDecision::AllowOnce,
        ),
        Err(AgentError::StaleGeneration)
    );
    assert!(tools.calls().is_empty());
    assert_eq!(
        service.resolve_approval(
            session_id,
            turn.id,
            turn.generation,
            request.request_id,
            ApprovalUserDecision::AllowOnce,
        ),
        Ok(())
    );
    assert_eq!(turn_task.await.unwrap().unwrap().final_answer, "started");
    assert_eq!(tools.calls().len(), 1);
    assert_eq!(tools.approved_calls(), 1);
    assert_eq!(
        service.resolve_approval(
            session_id,
            turn.id,
            turn.generation,
            request.request_id,
            ApprovalUserDecision::AllowOnce,
        ),
        Err(AgentError::TurnNotRunning)
    );
}

#[tokio::test]
async fn denied_approval_becomes_a_model_visible_tool_error_without_dispatch() {
    let first = request(vec![ModelInputItem::user_text("start it")]);
    let denied = hanafubuki_agent_tool_contracts::ToolOutput::structured(
        json!({ "error": "user denied the requested operation" }),
        true,
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
            output: denied,
        },
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first, mutation_tool_call_events()),
        FakeModelExchange::new(second, deterministic_assistant_events("not started")),
    ]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    let session_id = session.id;
    let executing = service.clone();
    let turn_task = tokio::spawn(async move {
        executing
            .start_turn(session_id, "start it".to_string())
            .await
    });
    let (turn_id, generation, request_id) = loop {
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
                break (turn.id, turn.generation, request.request_id);
            }
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    service
        .resolve_approval(
            session_id,
            turn_id,
            generation,
            request_id,
            ApprovalUserDecision::Deny,
        )
        .unwrap();
    assert_eq!(
        turn_task.await.unwrap().unwrap().final_answer,
        "not started"
    );
    assert!(tools.calls().is_empty());
}

#[tokio::test]
async fn cancelling_a_turn_aborts_pending_manual_permission_without_dispatch() {
    let model = FakeModelTransport::new([FakeModelExchange::new(
        request(vec![ModelInputItem::user_text("start it")]),
        mutation_tool_call_events(),
    )]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    let session_id = session.id;
    let executing = service.clone();
    let turn_task = tokio::spawn(async move {
        executing
            .start_turn(session_id, "start it".to_string())
            .await
    });
    let (turn_id, generation) = loop {
        let snapshot = test_session_view(&service, session_id).unwrap();
        if let Some(turn) = snapshot.runtime.turn {
            if service
                .events_since(session_id, None)
                .unwrap()
                .events
                .into_iter()
                .any(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. }))
            {
                break (turn.id, turn.generation);
            }
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    service
        .request_turn_cancellation(session_id, turn_id, generation)
        .await
        .unwrap();
    assert_eq!(turn_task.await.unwrap(), Err(AgentError::Cancelled));
    assert!(tools.calls().is_empty());
}

#[tokio::test]
async fn approval_timeout_does_not_dispatch_the_mutation() {
    let first = request(vec![ModelInputItem::user_text("start it")]);
    let timed_out = hanafubuki_agent_tool_contracts::ToolOutput::structured(
        json!({
            "error": "approval timed out before the operation was dispatched"
        }),
        true,
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
            output: timed_out,
        },
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first, mutation_tool_call_events()),
        FakeModelExchange::new(second, deterministic_assistant_events("timed out")),
    ]);
    let tools = FakeTools::new([]).await;
    let limits = AgentHardLimits {
        approval_timeout: Duration::from_millis(5),
        ..FIRST_BATCH_HARD_LIMITS
    };
    let service = AgentService::with_limits(Arc::new(model), tools.registries(), limits);
    let session = service.create_session(config()).unwrap();
    assert_eq!(
        service
            .start_turn(session.id, "start it".to_string())
            .await
            .unwrap()
            .final_answer,
        "timed out"
    );
    assert!(tools.calls().is_empty());
}

#[tokio::test]
async fn auto_review_uses_isolated_text_evidence_then_dispatches_and_persists_status() {
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        permission_review_events("allow"),
        deterministic_assistant_events("started"),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "control_instance",
        json!({ "instanceId": "instance-a", "action": "start" }),
        json!({ "operationId": "launch-1", "instanceId": "instance-a" }),
    )])
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();

    let result = service
        .start_turn(
            session.id,
            permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::AutoReview),
        )
        .await
        .unwrap();

    assert_eq!(result.final_answer, "started");
    assert_eq!(tools.approved_calls(), 1);
    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[1].tool_choice,
        hanafubuki_agent_model_api::ModelToolChoice::None
    );
    assert!(requests[1].tools.is_empty());
    assert_eq!(requests[1].input.len(), 1);
    assert!(requests[1].input.iter().all(|item| matches!(
        item,
        hanafubuki_agent_model_api::ModelInputItem::UserMessage { .. }
    )));
    let snapshot = test_session_view(&service, session.id).unwrap();
    assert!(snapshot.model_context.iter().any(|item| matches!(
        item,
        ConversationItem::PermissionReview {
            status: hanafubuki_agent_kernel::PermissionReviewStatus::Approved,
            ..
        }
    )));
    assert!(!service
        .events_since(session.id, None)
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. })));
}

#[tokio::test]
async fn auto_review_denial_fails_closed_without_manual_fallback_or_dispatch() {
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        permission_review_events("deny"),
        deterministic_assistant_events("not started"),
    ]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();

    let result = service
        .start_turn(
            session.id,
            permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::AutoReview),
        )
        .await
        .unwrap();

    assert_eq!(result.final_answer, "not started");
    assert!(tools.calls().is_empty());
    let events = service.events_since(session.id, None).unwrap().events;
    assert!(events.iter().any(|event| matches!(
        event.kind,
        AgentEventKind::PermissionReviewCompleted {
            status: hanafubuki_agent_kernel::PermissionReviewStatus::Denied,
            ..
        }
    )));
    assert!(!events
        .iter()
        .any(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. })));
}

#[tokio::test]
async fn auto_review_transport_failure_is_failed_without_exposing_provider_call_id() {
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        vec![Err(ModelError::Remote {
            status: Some(400),
            code: None,
            message: "No tool output found for function call call_provider_secret".to_string(),
            request_id: Some("request_provider_secret".to_string()),
        })],
        deterministic_assistant_events("not started"),
    ]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();

    let result = service
        .start_turn(
            session.id,
            permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::AutoReview),
        )
        .await
        .unwrap();

    assert_eq!(result.final_answer, "not started");
    assert!(tools.calls().is_empty());
    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    let next_request = serde_json::to_string(&requests[2].input).unwrap();
    assert!(next_request
        .contains("automatic permission review failed closed; the action was not executed"));
    assert!(!next_request.contains("call_provider_secret"));
    assert!(!next_request.contains("request_provider_secret"));
    assert!(test_session_view(&service, session.id)
        .unwrap()
        .model_context
        .iter()
        .any(|item| matches!(
            item,
            ConversationItem::PermissionReview {
                status: hanafubuki_agent_kernel::PermissionReviewStatus::Failed,
                assessment: None,
                completed_at: Some(_),
                ..
            }
        )));
}

#[tokio::test]
async fn three_consecutive_auto_review_denials_stop_the_turn() {
    let model = ScriptedEventModel::new([
        mutation_tool_call_events_for("denied-call-1"),
        permission_review_events("deny"),
        mutation_tool_call_events_for("denied-call-2"),
        permission_review_events("deny"),
        mutation_tool_call_events_for("denied-call-3"),
        permission_review_events("deny"),
    ]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();

    let result = service
        .start_turn(
            session.id,
            permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::AutoReview),
        )
        .await;

    assert!(matches!(
        result,
        Err(AgentError::Approval(message))
            if message.contains("three consecutive actions")
    ));
    assert!(tools.calls().is_empty());
    assert_eq!(model.requests().len(), 6);
    assert_eq!(
        test_session_view(&service, session.id)
            .unwrap()
            .runtime
            .turn
            .unwrap()
            .phase,
        AgentTurnPhase::Failed
    );
}

#[tokio::test]
async fn full_access_issues_exact_authorization_without_a_reviewer_or_pending_request() {
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        deterministic_assistant_events("started"),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "control_instance",
        json!({ "instanceId": "instance-a", "action": "start" }),
        json!({ "operationId": "launch-1", "instanceId": "instance-a" }),
    )])
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();

    service
        .start_turn(
            session.id,
            permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::FullAccess),
        )
        .await
        .unwrap();

    assert_eq!(tools.approved_calls(), 1);
    assert_eq!(model.requests().len(), 2);
    let events = service.events_since(session.id, None).unwrap().events;
    assert!(!events.iter().any(|event| matches!(
        event.kind,
        AgentEventKind::ApprovalRequested { .. }
            | AgentEventKind::PermissionReviewStarted { .. }
            | AgentEventKind::PermissionReviewCompleted { .. }
    )));
}

#[tokio::test]
async fn full_access_cannot_override_a_forbidden_requirement() {
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        deterministic_assistant_events("not started"),
    ]);
    let tools = FakeTools::new([])
        .await
        .with_forbidden_mutations("hard product boundary");
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();

    let result = service
        .start_turn(
            session.id,
            permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::FullAccess),
        )
        .await
        .unwrap();

    assert_eq!(result.final_answer, "not started");
    assert!(tools.calls().is_empty());
}

/// "Allow for this session" answers the later asks for the same tool in the
/// session without a prompt; a new Agent instance that recovers the session
/// starts without it and asks again.
#[tokio::test]
async fn allow_for_session_answers_later_asks_for_the_tool_until_the_agent_restarts() {
    let exchange = || {
        FakeRegisteredExchange::success(
            "control_instance",
            json!({ "instanceId": "instance-a", "action": "start" }),
            json!({ "operationId": "launch-1", "instanceId": "instance-a" }),
        )
    };
    let tools = FakeTools::new([exchange(), exchange(), exchange()]).await;
    let store = Arc::new(hanafubuki_agent_kernel::EphemeralAgentHistoryStore::default());
    let model = ScriptedEventModel::new([
        mutation_tool_call_events_for("call-first"),
        mutation_tool_call_events_for("call-second"),
        deterministic_assistant_events("started twice"),
    ]);
    let service =
        AgentService::with_history_store(Arc::new(model), tools.registries(), store.clone());
    let session = service.create_session(config()).unwrap();
    let session_id = session.id;
    let executing = service.clone();
    let turn_task = tokio::spawn(async move {
        executing
            .start_turn(
                session_id,
                permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::Manual),
            )
            .await
    });

    let (turn, request) = approval_request(&service, session_id, 0).await;
    assert!(request.offers_allow_for_session);
    service
        .resolve_approval(
            session_id,
            turn.id,
            turn.generation,
            request.request_id,
            ApprovalUserDecision::AllowForSession,
        )
        .unwrap();
    assert_eq!(
        turn_task.await.unwrap().unwrap().final_answer,
        "started twice"
    );
    assert_eq!(tools.approved_calls(), 2);
    let requested = |service: &AgentService| {
        service
            .events_since(session_id, None)
            .unwrap()
            .events
            .iter()
            .filter(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. }))
            .count()
    };
    assert_eq!(requested(&service), 1);

    let model = ScriptedEventModel::new([
        mutation_tool_call_events_for("call-third"),
        deterministic_assistant_events("asked again"),
    ]);
    let restarted =
        AgentService::without_default_model_with_history_store(tools.registries(), store);
    assert_eq!(
        restarted
            .recover_sessions_with_test_model(Arc::new(model))
            .unwrap()
            .restored,
        1
    );
    let executing = restarted.clone();
    let turn_task = tokio::spawn(async move {
        executing
            .start_turn(
                session_id,
                permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::Manual),
            )
            .await
    });
    let (turn, request) = approval_request(&restarted, session_id, 0).await;
    restarted
        .resolve_approval(
            session_id,
            turn.id,
            turn.generation,
            request.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    assert_eq!(
        turn_task.await.unwrap().unwrap().final_answer,
        "asked again"
    );
    assert_eq!(tools.approved_calls(), 3);
}
