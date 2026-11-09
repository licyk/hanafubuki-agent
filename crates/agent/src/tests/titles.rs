//! Automatic titles: the provisional title the submit command commits with
//! a session's first turn, and the generated one the `titles` plugin
//! replaces it with.

use super::*;
use hanafubuki_agent_contracts::RegistryService as _;

/// What the model answers both the turn and the title request with: the
/// turn and the title worker race for the fake server's responses, and this
/// answer serves either.
const GENERATED: &str = r#"{"title":"Generated title"}"#;
const PROMPT: &str = "  Explain   the task   center  ";
const PROVISIONAL: &str = "Explain the task center";

#[derive(Default)]
struct RecordingSink(Mutex<Vec<AgentEvent>>);

impl AgentEventSink for RecordingSink {
    fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl RecordingSink {
    fn titles(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match &event.kind {
                AgentEventKind::SessionTitleUpdated { title } => Some(title.value.clone()),
                _ => None,
            })
            .collect()
    }
}

fn user_turn(text: &str) -> AgentTurnRequest {
    AgentTurnRequest {
        text: text.to_string(),
        images: Vec::new(),
        skills: Vec::new(),
        host: Default::default(),
    }
}

/// A started service on `server` with a new session, the `titles` plugin
/// turned off and gone when `titles` is false.
async fn titled_service(
    name: &str,
    server: &FakeServer,
    titles: bool,
) -> (
    AgentApplicationService,
    TestPaths,
    AgentSessionRuntimeSnapshot,
) {
    let (service, paths) = configured_service(name, &server.base_url);
    service.start_plugins().await;
    if !titles {
        service
            .set_plugin_enabled("titles".into(), false)
            .await
            .unwrap();
        let observers = service.test_plugin_registries().observers;
        wait_until("the titles plugin stops", || {
            !observers
                .registry()
                .snapshot()
                .entries()
                .iter()
                .any(|entry| entry.owner() == "titles")
        })
        .await;
    }
    let session = service
        .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default)
        .await
        .unwrap();
    (service, paths, session)
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + TEST_WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_turn_end(service: &AgentApplicationService, session_id: AgentSessionId) {
    wait_until("the turn ends", || {
        service
            .snapshot(session_id)
            .unwrap()
            .turn
            .is_some_and(|turn| turn.phase.is_terminal())
    })
    .await;
}

fn title_of(service: &AgentApplicationService, session_id: AgentSessionId) -> AgentSessionTitle {
    service.snapshot(session_id).unwrap().title.unwrap()
}

fn finish(service: AgentApplicationService, paths: TestPaths) {
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The first user turn shows its provisional title with the turn, and the
/// generated one replaces it through the session's sink.
// The managed turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_users_first_turn_is_titled_provisionally_then_by_the_model() {
    let server = FakeServer::start(vec![
        (assistant_events(GENERATED), Duration::ZERO),
        (assistant_events(GENERATED), Duration::ZERO),
    ])
    .await;
    let (service, paths, session) = titled_service("titles-generated", &server, true).await;
    let sink = Arc::new(RecordingSink::default());

    service
        .submit_turn_with_sink(session.id, user_turn(PROMPT), sink.clone())
        .await
        .unwrap();
    assert_eq!(sink.titles().first().map(String::as_str), Some(PROVISIONAL));
    wait_until("the generated title lands", || {
        title_of(&service, session.id).value == "Generated title"
    })
    .await;
    wait_for_turn_end(&service, session.id).await;

    let title = title_of(&service, session.id);
    assert_eq!(title.source, AgentSessionTitleSource::Automatic);
    assert_eq!(sink.titles(), [PROVISIONAL, "Generated title"]);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    // The title request carries the turn's prompt as the user typed it.
    assert!(requests.iter().any(|request| {
        let body = String::from_utf8_lossy(&request.body);
        body.contains("Create a concise title") && body.contains("Explain   the task")
    }));
    finish(service, paths);
}

/// With `titles` off the provisional title still comes with the turn, and
/// no title is ever asked for or generated.
// The managed turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_titles_plugin_off_only_the_provisional_title_lands() {
    let server = FakeServer::start(vec![
        (assistant_events(GENERATED), Duration::ZERO),
        (assistant_events(GENERATED), Duration::ZERO),
    ])
    .await;
    let (service, paths, session) = titled_service("titles-off", &server, false).await;
    let sink = Arc::new(RecordingSink::default());

    service
        .submit_turn_with_sink(session.id, user_turn(PROMPT), sink.clone())
        .await
        .unwrap();
    wait_for_turn_end(&service, session.id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(title_of(&service, session.id).value, PROVISIONAL);
    assert_eq!(sink.titles(), [PROVISIONAL]);
    assert_eq!(server.requests().len(), 1);
    // The history store holds the title the admission committed.
    let listed = service
        .list_sessions_page(hanafubuki_agent_kernel::AgentSessionListRequest::default())
        .unwrap();
    let summary = listed
        .sessions
        .iter()
        .find(|summary| summary.session_id == session.id)
        .unwrap();
    assert_eq!(
        summary.title.as_ref().map(|title| title.value.as_str()),
        Some(PROVISIONAL)
    );
    finish(service, paths);
}

/// A programmatic turn neither takes a provisional title nor asks for one.
#[tokio::test]
async fn a_programmatic_turn_is_not_titled() {
    let server = FakeServer::start(vec![
        (assistant_events(GENERATED), Duration::ZERO),
        (assistant_events(GENERATED), Duration::ZERO),
    ])
    .await;
    let (service, paths, session) = titled_service("titles-programmatic", &server, true).await;

    service.start_turn(session.id, PROMPT.into()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(service.snapshot(session.id).unwrap().title, None);
    assert_eq!(server.requests().len(), 1);
    finish(service, paths);
}

/// A rename while the title is generated wins: the generated title, whose
/// compare-and-swap expects the provisional one, is dropped.
// The managed turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_rename_during_generation_wins_over_the_generated_title() {
    let delay = Duration::from_millis(400);
    let server = FakeServer::start(vec![
        (assistant_events(GENERATED), delay),
        (assistant_events(GENERATED), delay),
    ])
    .await;
    let (service, paths, session) = titled_service("titles-renamed", &server, true).await;
    let sink = Arc::new(RecordingSink::default());

    service
        .submit_turn_with_sink(session.id, user_turn(PROMPT), sink.clone())
        .await
        .unwrap();
    // Both answers are held back, so the title is still being generated.
    let renamed = loop {
        let current = service.snapshot(session.id).unwrap();
        match service.rename_session(
            AgentSessionTitleRenameRequest {
                session_id: session.id,
                generation: current.generation,
                revision: current.revision,
                title: "My own title".to_string(),
            },
            sink.as_ref(),
        ) {
            Ok(renamed) => break renamed,
            Err(error) if error.starts_with("stale") => continue,
            Err(error) => panic!("rename failed: {error}"),
        }
    };
    wait_until("the title request is sent", || server.requests().len() == 2).await;
    wait_for_turn_end(&service, session.id).await;
    tokio::time::sleep(delay + Duration::from_millis(300)).await;

    assert_eq!(renamed.title.as_ref().unwrap().value, "My own title");
    let title = title_of(&service, session.id);
    assert_eq!(title.value, "My own title");
    assert_eq!(title.source, AgentSessionTitleSource::User);
    assert!(!sink.titles().iter().any(|title| title == "Generated title"));
    finish(service, paths);
}
