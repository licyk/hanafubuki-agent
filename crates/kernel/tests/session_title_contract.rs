use std::sync::{Arc, Mutex};

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{
    AgentError, AgentEvent, AgentEventKind, AgentEventSink, AgentKernelRegistries, AgentService,
    AgentSessionConfig, AgentSessionId, AgentSessionMode, AgentSessionTitleRenameRequest,
    AgentSessionTitleSource, AgentTurnInput, AgentTurnOrigin, ModelCompactionCapability,
};
use hanafubuki_agent_model_api::{
    ModelError, ModelFuture, ModelPort, ModelRequest, ModelResult, ModelStream,
};

struct UnusedModel;

impl ModelPort for UnusedModel {
    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        Box::pin(async { Err(ModelError::Protocol("unused test model".to_string())) })
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<AgentEvent>>);

impl AgentEventSink for RecordingSink {
    fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn config() -> AgentSessionConfig {
    AgentSessionConfig {
        provider_id: "provider-1".to_string(),
        provider_protocol: hanafubuki_agent_kernel::ModelApiProtocol::Responses,
        model: "fixture".to_string(),
        instructions: String::new(),
        mode: AgentSessionMode::Default,
        reasoning_effort: None,
        context_window_tokens: None,
        input_capabilities: Default::default(),
        auto_compaction_enabled: true,
        auto_compact_threshold_percent: 80,
        compaction_capability: ModelCompactionCapability::LocalSummary,
        sampling_retry_policy: Default::default(),
        compaction_strategy: Default::default(),
    }
}

/// A user's turn that brings a provisional title.
fn titled_input(text: &str, title: &str) -> AgentTurnInput {
    let mut input = AgentTurnInput::from(text);
    input.origin = AgentTurnOrigin::User;
    input.provisional_title = Some(title.to_string());
    input
}

/// Admits a turn that brings `title`; the turn itself then fails on the
/// unused model, after its admission committed.
async fn admit_titled_turn(service: &AgentService, session_id: AgentSessionId, title: &str) {
    assert!(service
        .start_turn(session_id, titled_input("Initial question?", title))
        .await
        .is_err());
}

#[tokio::test]
async fn manual_rename_is_durable_and_wins_the_automatic_compare_and_swap() {
    let service = AgentService::new(Arc::new(UnusedModel), AgentKernelRegistries::default());
    let sink = RecordingSink::default();
    let initial = service.create_session(config()).unwrap();
    admit_titled_turn(&service, initial.id, "Initial question").await;
    let provisional = service.snapshot(initial.id).unwrap();
    assert_eq!(
        provisional.title.as_ref().unwrap().value,
        "Initial question"
    );
    assert_eq!(
        provisional.title.as_ref().unwrap().source,
        AgentSessionTitleSource::Automatic
    );

    let renamed = service
        .rename_session(
            AgentSessionTitleRenameRequest {
                session_id: initial.id,
                generation: provisional.generation,
                revision: provisional.revision,
                title: "  用户   指定标题  ".to_string(),
            },
            &sink,
        )
        .unwrap();
    assert_eq!(renamed.title.as_ref().unwrap().value, "用户 指定标题");
    assert_eq!(
        renamed.title.as_ref().unwrap().source,
        AgentSessionTitleSource::User
    );
    assert!(service
        .replace_automatic_session_title_if_expected(
            initial.id,
            "Initial question",
            "Generated title".to_string(),
        )
        .unwrap()
        .is_none());
    assert_eq!(service.snapshot(initial.id).unwrap().title, renamed.title);
    assert!(sink
        .0
        .lock()
        .unwrap()
        .iter()
        .all(|event| matches!(event.kind, AgentEventKind::SessionTitleUpdated { .. })));
}

#[test]
fn manual_rename_requires_the_exact_session_revision_and_a_bounded_title() {
    let service = AgentService::new(Arc::new(UnusedModel), AgentKernelRegistries::default());
    let sink = RecordingSink::default();
    let initial = service.create_session(config()).unwrap();
    let stale = service.rename_session(
        AgentSessionTitleRenameRequest {
            session_id: initial.id,
            generation: initial.generation,
            revision: hanafubuki_agent_kernel::AgentRevision::new(initial.revision.value() + 1),
            title: "Name".to_string(),
        },
        &sink,
    );
    assert_eq!(stale, Err(AgentError::StaleRevision));

    let empty = service.rename_session(
        AgentSessionTitleRenameRequest {
            session_id: initial.id,
            generation: initial.generation,
            revision: initial.revision,
            title: " \n ".to_string(),
        },
        &sink,
    );
    assert!(matches!(empty, Err(AgentError::InvalidInput(_))));
}

/// The provisional title is committed with the turn's admission: its event
/// follows the turn's start and its user message, and a session that has a
/// title keeps it.
#[tokio::test]
async fn admission_commits_the_provisional_title_after_the_turn_events() {
    let service = AgentService::new(Arc::new(UnusedModel), AgentKernelRegistries::default());
    let session = service.create_session(config()).unwrap();
    let sink = Arc::new(RecordingSink::default());

    service
        .submit_turn_with_sink(
            session.id,
            titled_input("Initial question?", "Initial question"),
            sink.clone(),
        )
        .unwrap();

    let admission = sink.0.lock().unwrap()[..3].to_vec();
    assert!(matches!(admission[0].kind, AgentEventKind::TurnStarted));
    assert!(matches!(
        admission[1].kind,
        AgentEventKind::HistoryCommitted { .. }
    ));
    let AgentEventKind::SessionTitleUpdated { title } = &admission[2].kind else {
        panic!("the third admission event is {:?}", admission[2].kind);
    };
    assert_eq!(title.value, "Initial question");
    assert_eq!(title.source, AgentSessionTitleSource::Automatic);
    assert_eq!(admission[2].turn_id, None);
    assert_eq!(admission[2].generation, admission[0].generation);
    assert_eq!(
        admission
            .iter()
            .map(|event| event.sequence.value())
            .collect::<Vec<_>>(),
        (0..3)
            .map(|offset| admission[0].sequence.value() + offset)
            .collect::<Vec<_>>()
    );

    // The next turn's provisional title does not replace the first one.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while service
        .snapshot(session.id)
        .unwrap()
        .turn
        .is_some_and(|turn| !turn.phase.is_terminal())
    {
        assert!(std::time::Instant::now() < deadline, "the turn never ended");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    admit_titled_turn(&service, session.id, "Second question").await;
    assert_eq!(
        service.snapshot(session.id).unwrap().title.unwrap().value,
        "Initial question"
    );
}

/// A generated title replacing the provisional one reaches the sink of the
/// session's managed turn.
#[tokio::test]
async fn a_generated_title_goes_to_the_sessions_retained_sink() {
    let service = AgentService::new(Arc::new(UnusedModel), AgentKernelRegistries::default());
    let session = service.create_session(config()).unwrap();
    let sink = Arc::new(RecordingSink::default());
    service
        .submit_turn_with_sink(
            session.id,
            titled_input("Initial question?", "Initial question"),
            sink.clone(),
        )
        .unwrap();

    let replaced = service
        .replace_automatic_session_title_if_expected(
            session.id,
            "Initial question",
            "Generated title".to_string(),
        )
        .unwrap()
        .unwrap();

    assert_eq!(replaced.title.as_ref().unwrap().value, "Generated title");
    assert!(sink.0.lock().unwrap().iter().any(|event| matches!(
        &event.kind,
        AgentEventKind::SessionTitleUpdated { title } if title.value == "Generated title"
    )));
}
