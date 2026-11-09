use super::*;

const APPROVED_NOTE: &str = "The user has manually approved a specific action that automatic permission review previously denied.\n\n\
Treat this as approval to perform that exact action in the same context in which it was originally requested.\n\
Do not assume this also authorizes similar operations with different payloads.\n\n\
Approved action:\n";

#[tokio::test]
async fn approving_a_denied_action_commits_a_note_the_model_and_the_next_review_read() {
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        permission_review_events("deny"),
        deterministic_assistant_events("not started"),
        mutation_tool_call_events_for("call-retry"),
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
    let auto_review =
        || permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::AutoReview);

    let denied = service.start_turn(session.id, auto_review()).await.unwrap();
    assert_eq!(denied.final_answer, "not started");
    let (review_id, action_summary) = denied_review(&service, session.id);
    let events_before = service.events_since(session.id, None).unwrap().events.len();

    service
        .approve_denied_action(session.id, &review_id)
        .unwrap();

    // No turn started: the note is the only change.
    assert_eq!(model.requests().len(), 3);
    let events = service.events_since(session.id, None).unwrap().events;
    assert!(matches!(
        &events[events_before..],
        [AgentEvent {
            turn_id: None,
            kind: AgentEventKind::HistoryCommitted { item_count: 1, .. },
            ..
        }]
    ));
    let expected = format!("{APPROVED_NOTE}{action_summary}");
    assert!(matches!(
        test_session_view(&service, session.id).unwrap().model_context.last(),
        Some(ConversationItem::ContinuationContext { content, .. }) if *content == expected
    ));

    let retried = service.start_turn(session.id, auto_review()).await.unwrap();
    assert_eq!(retried.final_answer, "started");
    assert_eq!(tools.approved_calls(), 1);
    let requests = model.requests();
    assert_eq!(requests.len(), 6);
    assert!(requests[3].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ContinuationContext { provider_id, hook_id, content, .. }
            if provider_id == "kernel"
                && hook_id == "approved-denied-action"
                && *content == expected
    )));
    // The retry is reviewed again, and the reviewer reads the approval.
    let review_prompt = user_text(&requests[4].input);
    assert!(review_prompt.contains(&format!("user approval: {expected}")));
}

#[tokio::test]
async fn only_a_denied_review_of_the_session_can_be_approved() {
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
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(
            session.id,
            permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::AutoReview),
        )
        .await
        .unwrap();
    let approved_review = service
        .events_since(session.id, None)
        .unwrap()
        .events
        .into_iter()
        .find_map(|event| match event.kind {
            AgentEventKind::PermissionReviewCompleted { review_id, .. } => Some(review_id),
            _ => None,
        })
        .unwrap();

    for review_id in [approved_review.as_str(), "permission-review-unknown"] {
        assert_eq!(
            service.approve_denied_action(session.id, review_id),
            Err(AgentError::ApprovalNotFound)
        );
    }
}

#[tokio::test]
async fn approving_while_a_turn_runs_is_refused() {
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

    let approving = service.clone();
    let refused = tokio::task::spawn_blocking(move || {
        approving.approve_denied_action(session_id, "permission-review-any")
    })
    .await
    .unwrap();

    assert_eq!(refused, Err(AgentError::TurnAlreadyRunning));
    service
        .request_turn_cancellation(session_id, turn_id, generation)
        .await
        .unwrap();
    assert_eq!(turn_task.await.unwrap(), Err(AgentError::Cancelled));
}

/// The id and action summary of the session's denied review.
fn denied_review(service: &AgentService, session_id: AgentSessionId) -> (String, String) {
    service
        .events_since(session_id, None)
        .unwrap()
        .events
        .into_iter()
        .find_map(|event| match event.kind {
            AgentEventKind::PermissionReviewCompleted {
                review_id,
                action_summary,
                status: hanafubuki_agent_kernel::PermissionReviewStatus::Denied,
                ..
            } => Some((review_id, action_summary)),
            _ => None,
        })
        .expect("the review was denied")
}

fn user_text(input: &[ModelInputItem]) -> String {
    input
        .iter()
        .filter_map(|item| match item {
            ModelInputItem::UserMessage { content } => Some(content),
            _ => None,
        })
        .flatten()
        .filter_map(|part| match part {
            hanafubuki_agent_model_api::ModelUserContent::InputText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}
