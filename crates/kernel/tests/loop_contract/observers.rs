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
                provisional_title: None,
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

/// `TurnAdmitted` carries who asked for the turn and the provisional title
/// its admission committed: the first user turn of an untitled session
/// gives it one, a later turn finds the title taken. It is told before
/// anything the turn commits.
#[tokio::test]
async fn an_admitted_turn_is_told_with_its_origin_and_provisional_title() {
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
                provisional_title: Some("First title".to_string()),
            },
            AgentNotification::TurnAdmitted {
                session_id: session.id,
                turn_id: second.turn_id,
                generation: second.generation,
                origin: AgentTurnOrigin::User,
                provisional_title: None,
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
    assert_eq!(
        service
            .turn_user_message(session.id, first.turn_id)
            .unwrap()
            .as_deref(),
        Some("first")
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
