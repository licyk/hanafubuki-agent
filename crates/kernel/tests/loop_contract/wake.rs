//! Wake events: queued on a session, delivered once as one `WakeEvents`
//! item at a turn's admission or at a step of the running turn, and the
//! turn they open by themselves.

use hanafubuki_agent_kernel::{
    wake_events_record, AgentExecutionProgressStatus, AgentSubagentWake, AgentTerminalWake,
    AgentTurnAccepted, AgentWakeEvent, AgentWakeFilter, AgentWakePolicy, AgentWakeQueued,
    AgentWakeSource, MAXIMUM_QUEUED_WAKE_MESSAGES,
};

use super::*;

fn completed(id: u64, answer: &str) -> AgentWakeSource {
    AgentWakeSource::Subagent {
        session_id: AgentSessionId::new(id),
        name: format!("agent_{id}"),
        event: AgentSubagentWake::Completed {
            answer: Some(answer.to_string()),
            truncated: false,
        },
    }
}

fn message(id: u64, text: &str) -> AgentWakeSource {
    AgentWakeSource::Subagent {
        session_id: AgentSessionId::new(id),
        name: format!("agent_{id}"),
        event: AgentSubagentWake::Message {
            text: text.to_string(),
        },
    }
}

fn exited(execution_id: u64) -> AgentWakeSource {
    AgentWakeSource::Terminal {
        execution_id,
        command: "cargo test".to_string(),
        event: AgentTerminalWake::Ended {
            status: AgentExecutionProgressStatus::Exited,
            exit_code: Some(0),
            duration_millis: 3_000,
            output: "ok".to_string(),
            truncated: false,
        },
    }
}

/// What the model reads of `sources`, delivered together.
fn fragment(sources: &[AgentWakeSource]) -> ModelInputItem {
    let events = sources
        .iter()
        .map(|source| AgentWakeEvent {
            sequence: 0,
            queued_at: 0,
            source: source.clone(),
        })
        .collect::<Vec<_>>();
    ModelInputItem::ContextFragment {
        role: ModelContextRole::User,
        kind: "background_events".to_string(),
        content: wake_events_record(&events),
    }
}

/// The sources of every `WakeEvents` item of the session's history.
fn delivered(service: &AgentService, session_id: AgentSessionId) -> Vec<Vec<AgentWakeSource>> {
    test_session_view(service, session_id)
        .unwrap()
        .model_context
        .iter()
        .filter_map(|item| match item {
            ConversationItem::WakeEvents { events } => Some(
                events
                    .iter()
                    .map(|event| event.source.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .collect()
}

async fn wake_session(service: &AgentService) -> AgentSessionId {
    let session = service.create_session(config()).unwrap();
    assert!(service
        .set_session_wake_policy(session.id, AgentWakePolicy::On)
        .await
        .unwrap());
    session.id
}

async fn wait_for_completion(service: &AgentService, accepted: AgentTurnAccepted) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let turn = service.snapshot(accepted.session_id).unwrap().turn;
            if turn.is_some_and(|turn| {
                turn.generation == accepted.generation && turn.phase == AgentTurnPhase::Completed
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the turn must complete");
}

#[tokio::test]
async fn an_event_queued_while_a_turn_runs_is_delivered_at_its_next_step() {
    let tool_output = hanafubuki_agent_tool_contracts::ToolOutput::structured(
        json!({ "instanceId": "instance-a" }),
        false,
    );
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        deterministic_assistant_events("started"),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "control_instance",
        json!({ "instanceId": "instance-a", "action": "start" }),
        tool_output,
    )])
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session_id = wake_session(&service).await;
    let executing = service.clone();
    let task = tokio::spawn(async move {
        executing
            .start_turn(session_id, "start it".to_string())
            .await
    });
    let (turn, approval) = approval_request(&service, session_id, 0).await;

    assert_eq!(
        service
            .enqueue_wake_event(session_id, completed(41, "found it"))
            .await,
        Ok(AgentWakeQueued::Delivering)
    );
    assert_eq!(service.snapshot(session_id).unwrap().pending_wake_events, 1);
    service
        .resolve_approval(
            session_id,
            turn.id,
            turn.generation,
            approval.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    assert_eq!(task.await.unwrap().unwrap().final_answer, "started");

    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    // The step after the tool's output reads the events, once.
    assert!(matches!(
        &requests[1].input[..],
        [.., ModelInputItem::ToolOutput { .. }, last] if *last == fragment(&[completed(41, "found it")])
    ));
    assert_eq!(
        delivered(&service, session_id),
        vec![vec![completed(41, "found it")]]
    );
    assert_eq!(service.snapshot(session_id).unwrap().pending_wake_events, 0);
}

/// Events queued after the last step's check (here, while its final
/// answer's continuation hook runs) keep the turn going one more step.
#[tokio::test]
async fn an_event_queued_after_the_last_step_takes_one_more_step() {
    let model = RecordingAssistantModel::default();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let tools = FakeTools::new([]).await;
    tools
        .mount(&registering::<ContinuationHookRegistry>(
            "finalization-gate",
            vec![AgentContinuationHookRegistration {
                provider_id: AgentContinuationHookProviderId::parse("test".to_string()).unwrap(),
                hook_id: AgentContinuationHookId::parse("gate".to_string()).unwrap(),
                generation: AgentContinuationHookGeneration([3; 32]),
                priority: 1,
                failure_policy: AgentContinuationHookFailurePolicy::FailTurn,
                hook: Arc::new(BlockingOnceHook {
                    blocked: AtomicBool::new(false),
                    entered: entered.clone(),
                    release: release.clone(),
                }),
            }],
        ))
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session_id = wake_session(&service).await;
    let executing = service.clone();
    let task =
        tokio::spawn(async move { executing.start_turn(session_id, "draft".to_string()).await });
    entered.wait().await;
    assert_eq!(
        service.enqueue_wake_event(session_id, exited(12)).await,
        Ok(AgentWakeQueued::Delivering)
    );
    // The running turn delivers it: no wake turn is admitted beside it.
    assert_eq!(
        service.submit_wake_turn_with_sink(
            session_id,
            AgentTurnInput::from(String::new()),
            Arc::new(NoopEventSink),
        ),
        Ok(None)
    );
    release.wait().await;

    assert_eq!(task.await.unwrap().unwrap().final_answer, "final");
    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].input,
        vec![
            ModelInputItem::user_text("draft"),
            ModelInputItem::AssistantMessage {
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: "initial".to_string(),
            },
            fragment(&[exited(12)]),
        ]
    );
    assert_eq!(delivered(&service, session_id).len(), 1);
}

/// A person's turn delivers what queued while the session was idle, as one
/// item before the message; later requests carry it as history.
#[tokio::test]
async fn an_admission_delivers_the_queue_before_the_message() {
    let model = RecordingAssistantModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session_id = wake_session(&service).await;
    for source in [completed(41, "first"), message(42, "a note"), exited(12)] {
        assert_eq!(
            service.enqueue_wake_event(session_id, source).await,
            Ok(AgentWakeQueued::Waiting)
        );
    }
    service
        .start_turn(session_id, "what happened?".to_string())
        .await
        .unwrap();
    service
        .start_turn(session_id, "and then?".to_string())
        .await
        .unwrap();

    let sources = [completed(41, "first"), message(42, "a note"), exited(12)];
    let requests = model.requests();
    assert_eq!(
        requests[0].input,
        vec![
            fragment(&sources),
            ModelInputItem::user_text("what happened?")
        ]
    );
    assert_eq!(
        requests[1].input,
        vec![
            fragment(&sources),
            ModelInputItem::user_text("what happened?"),
            ModelInputItem::AssistantMessage {
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: "initial".to_string(),
            },
            ModelInputItem::user_text("and then?"),
        ]
    );
    let history = test_session_view(&service, session_id)
        .unwrap()
        .model_context;
    assert!(matches!(
        &history[..3],
        [
            ConversationItem::TurnStart { .. },
            ConversationItem::WakeEvents { .. },
            ConversationItem::UserMessage { .. },
        ]
    ));
}

#[tokio::test]
async fn a_wake_turn_opens_with_the_events_alone() {
    let model = RecordingAssistantModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session_id = wake_session(&service).await;
    let wake = |service: &AgentService| {
        service.submit_wake_turn_with_sink(
            session_id,
            AgentTurnInput::from(String::new()),
            Arc::new(NoopEventSink),
        )
    };
    assert_eq!(wake(&service), Ok(None));
    service
        .enqueue_wake_event(session_id, completed(41, "done"))
        .await
        .unwrap();
    service
        .enqueue_wake_event(session_id, completed(43, "also done"))
        .await
        .unwrap();

    let accepted = wake(&service).unwrap().expect("a wake turn");
    wait_for_completion(&service, accepted).await;

    assert_eq!(
        model.requests()[0].input,
        vec![fragment(&[
            completed(41, "done"),
            completed(43, "also done")
        ])]
    );
    let history = test_session_view(&service, session_id)
        .unwrap()
        .model_context;
    assert!(matches!(
        &history[..],
        [
            ConversationItem::TurnStart { .. },
            ConversationItem::WakeEvents { .. },
            ConversationItem::AssistantMessage { .. },
        ]
    ));
    // Delivered once: the queue is empty, and a second wake is nothing.
    assert_eq!(wake(&service), Ok(None));
    assert_eq!(service.snapshot(session_id).unwrap().pending_wake_events, 0);
    assert_eq!(model.requests().len(), 1);
}

#[tokio::test]
async fn a_wake_turn_waits_for_the_running_turn_and_is_refused_with_the_policy_off() {
    let service = AgentService::new(
        Arc::new(CancellationOnlyModel),
        FakeTools::new([]).await.registries(),
    );
    let session_id = wake_session(&service).await;
    let executing = service.clone();
    let task =
        tokio::spawn(async move { executing.start_turn(session_id, "wait".to_string()).await });
    let turn = wait_for_active_turn(&service, session_id).await;
    assert_eq!(
        service
            .enqueue_wake_event(session_id, completed(41, "done"))
            .await,
        Ok(AgentWakeQueued::Delivering)
    );
    assert_eq!(
        service.submit_wake_turn_with_sink(
            session_id,
            AgentTurnInput::from(String::new()),
            Arc::new(NoopEventSink),
        ),
        Ok(None)
    );
    service
        .request_turn_cancellation(session_id, turn.id, turn.generation)
        .await
        .unwrap();
    assert_eq!(task.await.unwrap(), Err(AgentError::Cancelled));

    // The cancelled turn took no step: the event is still queued.
    assert_eq!(service.snapshot(session_id).unwrap().pending_wake_events, 1);
    assert!(service
        .set_session_wake_policy(session_id, AgentWakePolicy::Off)
        .await
        .unwrap());
    assert!(matches!(
        service.submit_wake_turn_with_sink(
            session_id,
            AgentTurnInput::from(String::new()),
            Arc::new(NoopEventSink),
        ),
        Err(AgentError::InvalidInput(_))
    ));
    assert_eq!(service.snapshot(session_id).unwrap().pending_wake_events, 1);
}

/// With the policy off, nothing is delivered: a sub-agent's final status
/// stays queued for `wait_agent`, and anything else is dropped.
#[tokio::test]
async fn with_the_policy_off_only_a_subagents_status_is_kept_and_nothing_delivered() {
    let model = RecordingAssistantModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    assert_eq!(
        service.snapshot(session.id).unwrap().wake_policy,
        AgentWakePolicy::Off
    );
    assert_eq!(
        service.enqueue_wake_event(session.id, exited(12)).await,
        Ok(AgentWakeQueued::Disabled)
    );
    assert_eq!(
        service
            .enqueue_wake_event(session.id, message(41, "hello"))
            .await,
        Ok(AgentWakeQueued::Disabled)
    );
    assert_eq!(
        service
            .enqueue_wake_event(session.id, completed(41, "done"))
            .await,
        Ok(AgentWakeQueued::Waiting)
    );
    service
        .start_turn(session.id, "go".to_string())
        .await
        .unwrap();
    assert_eq!(
        model.requests()[0].input,
        vec![ModelInputItem::user_text("go")]
    );
    assert!(delivered(&service, session.id).is_empty());
    assert_eq!(
        service
            .take_wake_events(session.id, AgentWakeFilter::SubagentStatuses)
            .await
            .unwrap()
            .into_iter()
            .map(|event| event.source)
            .collect::<Vec<_>>(),
        vec![completed(41, "done")]
    );
    assert_eq!(service.snapshot(session.id).unwrap().pending_wake_events, 0);
}

#[tokio::test]
async fn take_wake_events_removes_what_its_filter_names() {
    let service = AgentService::new(
        Arc::new(RecordingAssistantModel::default()),
        FakeTools::new([]).await.registries(),
    );
    let session_id = wake_session(&service).await;
    for source in [
        completed(41, "done"),
        message(41, "note"),
        exited(12),
        exited(13),
    ] {
        service
            .enqueue_wake_event(session_id, source)
            .await
            .unwrap();
    }
    let take = |filter| service.take_wake_events(session_id, filter);
    let sources = |events: Vec<AgentWakeEvent>| {
        events
            .into_iter()
            .map(|event| event.source)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        sources(
            take(AgentWakeFilter::Terminal { execution_id: 12 })
                .await
                .unwrap()
        ),
        vec![exited(12)]
    );
    assert_eq!(
        sources(take(AgentWakeFilter::SubagentStatuses).await.unwrap()),
        vec![completed(41, "done")]
    );
    assert!(take(AgentWakeFilter::Terminal { execution_id: 12 })
        .await
        .unwrap()
        .is_empty());
    assert_eq!(service.snapshot(session_id).unwrap().pending_wake_events, 2);

    // Unread messages are bounded; final statuses still queue.
    for index in 1..MAXIMUM_QUEUED_WAKE_MESSAGES {
        service
            .enqueue_wake_event(session_id, message(41, &index.to_string()))
            .await
            .unwrap();
    }
    assert_eq!(
        service
            .enqueue_wake_event(session_id, message(41, "one too many"))
            .await,
        Err(AgentError::LimitExceeded {
            resource: "unread wake messages",
            limit: MAXIMUM_QUEUED_WAKE_MESSAGES,
        })
    );
    assert_eq!(
        service.enqueue_wake_event(session_id, exited(14)).await,
        Ok(AgentWakeQueued::Waiting)
    );
}
