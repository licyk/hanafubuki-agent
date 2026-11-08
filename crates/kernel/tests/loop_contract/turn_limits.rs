//! The turn's own limits: after `maximum_sampling_steps` samples the turn
//! fails with that limit, and at `turn_deadline` it is cancelled as a user's
//! cancel would, committing what its calls finished, and fails naming the
//! deadline.

use super::*;

fn tool_outputs(service: &AgentService, session_id: AgentSessionId) -> Vec<(String, bool)> {
    test_session_view(service, session_id)
        .unwrap()
        .model_context
        .into_iter()
        .filter_map(|item| match item {
            ConversationItem::ToolOutput {
                call_id, output, ..
            } => Some((call_id, format!("{output:?}").contains("interrupted"))),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_turn_ends_after_its_maximum_sampling_steps_with_its_outputs_committed() {
    let model = ScriptedEventModel::new([
        calls_events(&[("call-1", "list_instances", json!({}))]),
        calls_events(&[("call-2", "list_instances", json!({}))]),
        deterministic_assistant_events("never sampled"),
    ]);
    let tools = FakeTools::new([
        FakeRegisteredExchange::success("list_instances", json!({}), json!({ "instances": [] })),
        FakeRegisteredExchange::success("list_instances", json!({}), json!({ "instances": [] })),
    ])
    .await;
    let limits = AgentHardLimits {
        maximum_sampling_steps: 2,
        ..FIRST_BATCH_HARD_LIMITS
    };
    let service = AgentService::with_limits(Arc::new(model.clone()), tools.registries(), limits);
    let session = service.create_session(config()).unwrap();

    let error = service
        .start_turn(session.id, "list".to_string())
        .await
        .unwrap_err();

    assert_eq!(
        error,
        AgentError::LimitExceeded {
            resource: "sampling steps",
            limit: 2,
        }
    );
    assert_eq!(model.requests().len(), 2);
    let turn = service.snapshot(session.id).unwrap().turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Failed);
    assert_eq!(
        turn.terminal_reason,
        Some(AgentTurnTerminalReason::BudgetExceeded)
    );
    assert_eq!(
        turn.last_error.as_deref(),
        Some("agent sampling steps limit exceeded (2)")
    );
    assert_eq!(
        tool_outputs(&service, session.id),
        [("call-1".to_string(), false), ("call-2".to_string(), false)]
    );
}

/// The first call waits for the user's approval, holding none of the
/// step's gate, so the second runs and finishes meanwhile. At the deadline
/// the approval is withdrawn: the finished call's output is committed and
/// the waiting one is interrupted.
#[tokio::test]
async fn a_turn_past_its_deadline_is_stopped_with_its_finished_calls_committed() {
    let model = ScriptedEventModel::new([calls_events(&[
        (
            "call-mutation",
            "control_instance",
            json!({ "instanceId": "instance-a", "action": "start" }),
        ),
        ("call-read", "list_instances", json!({})),
    ])]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        json!({ "instances": [] }),
    )])
    .await;
    let limits = AgentHardLimits {
        turn_deadline: Duration::from_millis(300),
        ..FIRST_BATCH_HARD_LIMITS
    };
    let service = AgentService::with_limits(Arc::new(model.clone()), tools.registries(), limits);
    let session = service.create_session(config()).unwrap();

    let error = service
        .start_turn(
            session.id,
            permission_turn_input(hanafubuki_agent_kernel::AgentPermissionMode::Manual),
        )
        .await
        .unwrap_err();

    assert_eq!(
        error,
        AgentError::TurnDeadlineExceeded {
            deadline: Duration::from_millis(300)
        }
    );
    assert_eq!(tools.calls(), [("list_instances".to_string(), json!({}))]);
    let turn = service.snapshot(session.id).unwrap().turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Failed);
    assert_eq!(
        turn.terminal_reason,
        Some(AgentTurnTerminalReason::DeadlineExceeded)
    );
    assert_eq!(
        turn.last_error.as_deref(),
        Some("agent turn stopped at its time limit of 0.3 s; send a message to continue")
    );
    let events = service.events_since(session.id, None).unwrap().events;
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        AgentEventKind::ApprovalResolved { resolution, .. }
            if resolution.decision == hanafubuki_agent_kernel::ApprovalDecision::Aborted
    )));
    assert_eq!(
        tool_outputs(&service, session.id),
        [
            ("call-read".to_string(), false),
            ("call-mutation".to_string(), true)
        ]
    );
}

/// A turn the user cancelled stays the user's cancel when its deadline
/// passes while a call still drains.
#[tokio::test]
async fn a_deadline_passing_after_a_user_cancel_keeps_the_cancel() {
    let model =
        ScriptedEventModel::new([calls_events(&[("call-read", "list_instances", json!({}))])]);
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let tools = FakeTools::new([])
        .await
        .with_invoke_gate(entered.clone(), release.clone());
    let limits = AgentHardLimits {
        turn_deadline: Duration::from_millis(100),
        ..FIRST_BATCH_HARD_LIMITS
    };
    let service = AgentService::with_limits(Arc::new(model), tools.registries(), limits);
    let session = service.create_session(config()).unwrap();
    let session_id = session.id;
    let executing = service.clone();
    let turn_task =
        tokio::spawn(async move { executing.start_turn(session_id, "list".to_string()).await });

    entered.wait().await;
    let turn = service.snapshot(session_id).unwrap().turn.unwrap();
    service
        .request_turn_cancellation(session_id, turn.id, turn.generation)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    release.wait().await;

    assert_eq!(turn_task.await.unwrap(), Err(AgentError::Cancelled));
    let turn = service.snapshot(session_id).unwrap().turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Cancelled);
    assert_eq!(
        turn.terminal_reason,
        Some(AgentTurnTerminalReason::UserInterrupted)
    );
}
