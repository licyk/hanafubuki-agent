use std::sync::OnceLock;

use hanafubuki_agent_contracts::{
    AgentMemoryContributionCause, AgentNotification, AgentObserver, AgentTurnOrigin,
    ObserverRegistry,
};
use hanafubuki_agent_kernel::{AgentError, AgentMemoryContributionMode, AgentSessionLifecycle};

use super::*;

/// Records what it is told and, when a turn finishes or a session is
/// archived or restored, the turn phase or session lifecycle the kernel has
/// committed at that moment.
#[derive(Default)]
struct RecordingObserver {
    service: OnceLock<AgentService>,
    seen: Mutex<Vec<AgentNotification>>,
    finished_phases: Mutex<Vec<AgentTurnPhase>>,
    lifecycles: Mutex<Vec<AgentSessionLifecycle>>,
}

impl AgentObserver for RecordingObserver {
    fn id(&self) -> &str {
        "recording"
    }

    fn observe(&self, notification: &AgentNotification) {
        if let AgentNotification::TurnFinished { session_id, .. } = notification {
            let service = self.service.get().expect("the service is bound");
            let turn = service.snapshot(*session_id).unwrap().turn.unwrap();
            self.finished_phases.lock().unwrap().push(turn.phase);
        }
        if let AgentNotification::SessionArchived { session_id, .. }
        | AgentNotification::SessionRestored { session_id, .. } = notification
        {
            let service = self.service.get().expect("the service is bound");
            let lifecycle = service.snapshot(*session_id).unwrap().lifecycle;
            self.lifecycles.lock().unwrap().push(lifecycle);
        }
        self.seen.lock().unwrap().push(notification.clone());
    }
}

fn answering(prompt: &str, answer: &str) -> FakeModelExchange {
    FakeModelExchange::new(
        request(vec![ModelInputItem::user_text(prompt)]),
        deterministic_assistant_events(answer),
    )
}

#[tokio::test]
async fn observers_are_told_what_the_kernel_committed_after_it_is_durable() {
    let model = FakeModelTransport::new([answering("inspect", "done")]);
    let tools = FakeTools::new([]).await;
    let observer = Arc::new(RecordingObserver::default());
    tools
        .mount(&registering::<ObserverRegistry>(
            "recording",
            vec![observer.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let _ = observer.service.set(service.clone());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "inspect".to_string())
        .await
        .unwrap();
    let contribution = service
        .set_session_memory_contribution(session.id, session.memory_contribution.revision, true)
        .unwrap();

    assert_eq!(
        *observer.seen.lock().unwrap(),
        [
            AgentNotification::TurnAdmitted {
                session_id: session.id,
                turn_id: outcome.turn_id,
                generation: outcome.generation,
                origin: AgentTurnOrigin::Programmatic,
                automatic_title: None,
            },
            AgentNotification::SamplingCommitted {
                session_id: session.id,
                turn_id: outcome.turn_id,
                generation: outcome.generation,
                memory_citations: Vec::new(),
            },
            AgentNotification::TurnFinished {
                session_id: session.id,
                turn_id: outcome.turn_id,
                generation: outcome.generation,
            },
            AgentNotification::MemoryContributionChanged {
                session_id: session.id,
                state: contribution,
                cause: AgentMemoryContributionCause::User,
                revision: service.snapshot(session.id).unwrap().revision,
            },
        ]
    );
    assert_eq!(contribution.mode, AgentMemoryContributionMode::Enabled);
    // The terminal state was committed, and the registry lock released,
    // before the observer ran.
    assert_eq!(
        *observer.finished_phases.lock().unwrap(),
        [AgentTurnPhase::Completed]
    );
}

/// The contribution a session is created with and the user's toggle are
/// both committed and told, each with its cause.
#[tokio::test]
async fn a_memory_contribution_change_is_told_with_its_cause() {
    let tools = FakeTools::new([]).await;
    let observer = Arc::new(RecordingObserver::default());
    tools
        .mount(&registering::<ObserverRegistry>(
            "recording",
            vec![observer.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    let service = AgentService::new(Arc::new(FakeModelTransport::new([])), tools.registries());
    let _ = observer.service.set(service.clone());
    let session = service.create_session(config()).unwrap();

    let initial = service
        .initialize_session_memory_contribution(
            session.id,
            session.memory_contribution.revision,
            true,
        )
        .unwrap();
    let initial_revision = service.snapshot(session.id).unwrap().revision;
    let toggled = service
        .set_session_memory_contribution(session.id, initial.revision, false)
        .unwrap();
    // Asking for the state the session already has commits nothing.
    service
        .set_session_memory_contribution(session.id, toggled.revision, false)
        .unwrap();

    assert_eq!(
        *observer.seen.lock().unwrap(),
        [
            AgentNotification::MemoryContributionChanged {
                session_id: session.id,
                state: initial,
                cause: AgentMemoryContributionCause::Initial,
                revision: initial_revision,
            },
            AgentNotification::MemoryContributionChanged {
                session_id: session.id,
                state: toggled,
                cause: AgentMemoryContributionCause::User,
                revision: service.snapshot(session.id).unwrap().revision,
            },
        ]
    );
    assert_eq!(initial.mode, AgentMemoryContributionMode::Enabled);
    assert_eq!(toggled.mode, AgentMemoryContributionMode::Disabled);
}

#[tokio::test]
async fn archiving_and_restoring_a_session_are_told_after_they_are_committed() {
    let tools = FakeTools::new([]).await;
    let observer = Arc::new(RecordingObserver::default());
    tools
        .mount(&registering::<ObserverRegistry>(
            "recording",
            vec![observer.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    let service = AgentService::new(Arc::new(FakeModelTransport::new([])), tools.registries());
    let _ = observer.service.set(service.clone());
    let session = service.create_session(config()).unwrap();

    let archived = service
        .archive_session(session.id, session.generation, session.revision)
        .unwrap();
    // A stale archive commits nothing and is not told.
    assert!(matches!(
        service.archive_session(session.id, session.generation, session.revision),
        Err(AgentError::StaleRevision)
    ));
    let restored = service
        .restore_session(session.id, archived.generation, archived.revision)
        .unwrap();

    assert_eq!(
        *observer.seen.lock().unwrap(),
        [
            AgentNotification::SessionArchived {
                session_id: session.id,
                revision: archived.revision,
            },
            AgentNotification::SessionRestored {
                session_id: session.id,
                revision: restored.revision,
            },
        ]
    );
    // Each lifecycle was committed, and the session lock released, before
    // the observer ran.
    assert_eq!(
        *observer.lifecycles.lock().unwrap(),
        [
            AgentSessionLifecycle::Archived,
            AgentSessionLifecycle::Available
        ]
    );
}

#[tokio::test]
async fn a_failed_turn_is_finished_too() {
    let tools = FakeTools::new([]).await;
    let observer = Arc::new(RecordingObserver::default());
    tools
        .mount(&registering::<ObserverRegistry>(
            "recording",
            vec![observer.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    let service = AgentService::new(Arc::new(FakeModelTransport::new([])), tools.registries());
    let _ = observer.service.set(service.clone());
    let session = service.create_session(config()).unwrap();

    assert!(service
        .start_turn(session.id, "inspect".to_string())
        .await
        .is_err());

    assert_eq!(
        *observer.finished_phases.lock().unwrap(),
        [AgentTurnPhase::Failed]
    );
}

#[tokio::test]
async fn an_observer_stopped_with_its_plugin_is_no_longer_told() {
    let model = FakeModelTransport::new([
        answering("first", "one"),
        FakeModelExchange::new(
            request(vec![
                ModelInputItem::user_text("first"),
                ModelInputItem::AssistantMessage {
                    phase: Some(ModelMessagePhase::FinalAnswer),
                    text: "one".to_string(),
                },
                ModelInputItem::user_text("second"),
            ]),
            deterministic_assistant_events("two"),
        ),
    ]);
    let tools = FakeTools::new([]).await;
    let observer = Arc::new(RecordingObserver::default());
    let plugin = tools
        .mount(&registering::<ObserverRegistry>(
            "recording",
            vec![observer.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let _ = observer.service.set(service.clone());
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(session.id, "first".to_string())
        .await
        .unwrap();
    let told = observer.seen.lock().unwrap().len();
    assert_eq!(told, 3);

    plugin.dispose().await.unwrap();
    service
        .start_turn(session.id, "second".to_string())
        .await
        .unwrap();

    assert_eq!(observer.seen.lock().unwrap().len(), told);
    assert!(tools
        .registries()
        .observers
        .registry()
        .snapshot()
        .is_empty());
}

fn user_turn(text: &str, provisional_title: &str) -> AgentTurnInput {
    let mut input = AgentTurnInput::from(text);
    input.origin = AgentTurnOrigin::User;
    input.provisional_title = Some(provisional_title.to_string());
    input
}

/// `TurnAdmitted` carries who asked for the turn and the session's
/// automatic title after the admission: the first user turn of an untitled
/// session gives it its provisional one, a later turn finds it kept (its
/// own provisional title is not taken). It is told before anything the
/// turn commits.
#[tokio::test]
async fn an_admitted_turn_is_told_with_its_origin_and_automatic_title() {
    let model = FakeModelTransport::new([
        answering("first", "one"),
        FakeModelExchange::new(
            request(vec![
                ModelInputItem::user_text("first"),
                ModelInputItem::AssistantMessage {
                    phase: Some(ModelMessagePhase::FinalAnswer),
                    text: "one".to_string(),
                },
                ModelInputItem::user_text("second"),
            ]),
            deterministic_assistant_events("two"),
        ),
    ]);
    let tools = FakeTools::new([]).await;
    let observer = Arc::new(RecordingObserver::default());
    tools
        .mount(&registering::<ObserverRegistry>(
            "recording",
            vec![observer.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let _ = observer.service.set(service.clone());
    let session = service.create_session(config()).unwrap();

    let first = service
        .start_turn(session.id, user_turn("first", "First title"))
        .await
        .unwrap();
    let second = service
        .start_turn(session.id, user_turn("second", "Second title"))
        .await
        .unwrap();

    let admitted = observer
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|notification| matches!(notification, AgentNotification::TurnAdmitted { .. }))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        admitted,
        [
            AgentNotification::TurnAdmitted {
                session_id: session.id,
                turn_id: first.turn_id,
                generation: first.generation,
                origin: AgentTurnOrigin::User,
                automatic_title: Some("First title".to_string()),
            },
            // The session keeps the title the Agent gave it.
            AgentNotification::TurnAdmitted {
                session_id: session.id,
                turn_id: second.turn_id,
                generation: second.generation,
                origin: AgentTurnOrigin::User,
                automatic_title: Some("First title".to_string()),
            },
        ]
    );
    assert!(matches!(
        observer.seen.lock().unwrap().first(),
        Some(AgentNotification::TurnAdmitted { .. })
    ));
    assert_eq!(
        service.snapshot(session.id).unwrap().title.unwrap().value,
        "First title"
    );
}

/// A refused admission commits nothing and tells nothing, whichever path
/// submitted it.
#[tokio::test]
async fn a_refused_admission_is_not_told() {
    let tools = FakeTools::new([]).await;
    let observer = Arc::new(RecordingObserver::default());
    tools
        .mount(&registering::<ObserverRegistry>(
            "recording",
            vec![observer.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    let service = AgentService::new(Arc::new(FakeModelTransport::new([])), tools.registries());
    let _ = observer.service.set(service.clone());
    let session = service.create_session(config()).unwrap();

    assert!(matches!(
        service
            .start_turn(session.id, user_turn(" ", "Blank"))
            .await,
        Err(AgentError::InvalidInput(_))
    ));
    service
        .archive_session(session.id, session.generation, session.revision)
        .unwrap();
    observer.seen.lock().unwrap().clear();
    assert!(matches!(
        service
            .start_turn(session.id, user_turn("inspect", "Archived"))
            .await,
        Err(AgentError::SessionArchived)
    ));
    assert!(matches!(
        service.submit_turn_with_sink(
            session.id,
            user_turn("inspect", "Archived"),
            Arc::new(NoopEventSink),
        ),
        Err(AgentError::SessionArchived)
    ));

    assert!(observer.seen.lock().unwrap().is_empty());
    assert_eq!(service.snapshot(session.id).unwrap().title, None);
}

/// A wake event is told once it is queued: never one the policy dropped,
/// nor one an archived session refused.
#[tokio::test]
async fn observers_are_told_of_a_queued_wake_event() {
    let tools = FakeTools::new([]).await;
    let observer = Arc::new(RecordingObserver::default());
    tools
        .mount(&registering::<ObserverRegistry>(
            "recording",
            vec![observer.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    let model = FakeModelTransport::new([]);
    let service = AgentService::new(Arc::new(model), tools.registries());
    let _ = observer.service.set(service.clone());
    let session = service.create_session(config()).unwrap();
    let message = hanafubuki_agent_kernel::AgentWakeSource::Subagent {
        session_id: AgentSessionId::new(41),
        name: "agent_41".to_string(),
        event: hanafubuki_agent_kernel::AgentSubagentWake::Message {
            text: "a note".to_string(),
        },
    };

    // The policy is off: a message is dropped, untold.
    service
        .enqueue_wake_event(session.id, message.clone())
        .await
        .unwrap();
    assert!(observer.seen.lock().unwrap().is_empty());
    service
        .set_session_wake_policy(session.id, hanafubuki_agent_kernel::AgentWakePolicy::On)
        .await
        .unwrap();
    service
        .enqueue_wake_event(session.id, message.clone())
        .await
        .unwrap();
    assert_eq!(
        *observer.seen.lock().unwrap(),
        vec![AgentNotification::WakeEventQueued {
            session_id: session.id
        }]
    );
    let session = service.snapshot(session.id).unwrap();
    service
        .archive_session(session.id, session.generation, session.revision)
        .unwrap();
    observer.seen.lock().unwrap().clear();
    assert_eq!(
        service.enqueue_wake_event(session.id, message).await,
        Err(AgentError::SessionArchived)
    );
    assert!(observer.seen.lock().unwrap().is_empty());
}

/// Events that leave a wake queue are told once that is durable: delivered
/// at a step of the running turn, taken by a filter (a take that finds
/// nothing tells nothing), delivered at a turn's admission (after its
/// `TurnAdmitted`).
#[tokio::test]
async fn observers_are_told_when_events_leave_a_wake_queue() {
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        deterministic_assistant_events("started"),
        deterministic_assistant_events("read"),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "control_instance",
        json!({ "instanceId": "instance-a", "action": "start" }),
        hanafubuki_agent_tool_contracts::ToolOutput::structured(
            json!({ "instanceId": "instance-a" }),
            false,
        ),
    )])
    .await;
    let observer = Arc::new(RecordingObserver::default());
    tools
        .mount(&registering::<ObserverRegistry>(
            "recording",
            vec![observer.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let _ = observer.service.set(service.clone());
    let session_id = service.create_session(config()).unwrap().id;
    service
        .set_session_wake_policy(session_id, hanafubuki_agent_kernel::AgentWakePolicy::On)
        .await
        .unwrap();
    let left = || {
        observer
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| **seen == AgentNotification::BackgroundWorkChanged { session_id })
            .count()
    };
    let status = |id: u64| hanafubuki_agent_kernel::AgentWakeSource::Subagent {
        session_id: AgentSessionId::new(id),
        name: format!("agent_{id}"),
        event: hanafubuki_agent_kernel::AgentSubagentWake::Completed {
            answer: Some("done".to_string()),
            truncated: false,
        },
    };

    let executing = service.clone();
    let task = tokio::spawn(async move {
        executing
            .start_turn(session_id, "start it".to_string())
            .await
    });
    let (turn, approval) = approval_request(&service, session_id, 0).await;
    service
        .enqueue_wake_event(session_id, status(41))
        .await
        .unwrap();
    assert_eq!(left(), 0, "queued, not left");
    service
        .resolve_approval(
            session_id,
            turn.id,
            turn.generation,
            approval.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(left(), 1, "a step delivered it");

    service
        .enqueue_wake_event(session_id, status(42))
        .await
        .unwrap();
    let filter = hanafubuki_agent_kernel::AgentWakeFilter::SubagentStatus {
        session_id: AgentSessionId::new(43),
    };
    assert!(service
        .take_wake_events(session_id, filter)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(left(), 1, "nothing was taken");
    service
        .take_wake_events(
            session_id,
            hanafubuki_agent_kernel::AgentWakeFilter::SubagentStatuses,
        )
        .await
        .unwrap();
    assert_eq!(left(), 2, "taken");

    service
        .enqueue_wake_event(session_id, status(44))
        .await
        .unwrap();
    service
        .start_turn(session_id, "what happened?".to_string())
        .await
        .unwrap();
    assert_eq!(left(), 3, "an admission delivered it");
    let seen = observer.seen.lock().unwrap().clone();
    let admitted = seen
        .iter()
        .rposition(|seen| matches!(seen, AgentNotification::TurnAdmitted { .. }))
        .unwrap();
    assert_eq!(
        seen[admitted + 1],
        AgentNotification::BackgroundWorkChanged { session_id }
    );
}
