//! Wake events through the facade, on the fake model server: queued by
//! test support (nothing in the Agent queues one yet), delivered once as a
//! `<background_events>` fragment by a wake turn, by a person's turn or at
//! the running turn's next step, and kept across a cold reopen.

use hanafubuki_agent_kernel::{
    AgentContextRequest, AgentEvent, AgentEventSink, AgentHistorySortDirection,
    AgentTranscriptPageRequest, AgentTurnAccepted, ConversationItem,
};
use serde_json::Value;

use super::*;
use crate::{AgentSubagentWake, AgentWakePolicy, AgentWakeQueued, AgentWakeSource};

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

struct Discard;

impl AgentEventSink for Discard {
    fn emit(&self, _event: AgentEvent) {}
}

/// A session of `service` whose wake events are delivered, with `sources`
/// queued.
pub(super) async fn session_with_events(
    service: &AgentApplicationService,
    sources: Vec<AgentWakeSource>,
) -> AgentSessionId {
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    set_wake_policy(service, session.id, AgentWakePolicy::On)
        .await
        .unwrap();
    for source in sources {
        assert_eq!(
            queue_wake_event(service, session.id, source).await,
            Ok(AgentWakeQueued::Waiting)
        );
    }
    session.id
}

/// Submits the session's wake turn and waits for its end.
pub(super) async fn run_wake_turn(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) -> AgentTurnAccepted {
    let accepted = service
        .submit_wake_turn_with_sink(session_id, Arc::new(Discard))
        .await
        .unwrap()
        .expect("events are queued");
    wait_for_end(service, &accepted).await;
    accepted
}

async fn wait_for_end(service: &AgentApplicationService, accepted: &AgentTurnAccepted) {
    tokio::time::timeout(TEST_WAIT, async {
        loop {
            let ended = service
                .events_since(accepted.session_id, None)
                .unwrap()
                .events
                .iter()
                .any(|event| event.turn_id == Some(accepted.turn_id) && event.kind.is_terminal());
            if ended {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the turn must end");
}

fn transcript(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) -> Vec<ConversationItem> {
    service
        .read_transcript_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 100,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.content)
        .collect()
}

/// How many times a request's input carries the fragment.
fn fragments(body: &[u8]) -> usize {
    let body: Value = serde_json::from_slice(body).unwrap();
    body["input"]
        .to_string()
        .matches("<background_events>")
        .count()
}

fn request_text(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body).unwrap()["input"].to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_session_is_woken_and_reads_its_events_once() {
    let server = FakeServer::start(vec![
        (assistant_events("Both are done."), Duration::ZERO),
        (assistant_events("Nothing new."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("wake-idle", &server.base_url);
    let session_id = session_with_events(
        &service,
        vec![
            completed(41, "first answer"),
            completed(42, "second answer"),
        ],
    )
    .await;

    run_wake_turn(&service, session_id).await;
    // The queue is drained: another wake is nothing.
    assert_eq!(
        service
            .submit_wake_turn_with_sink(session_id, Arc::new(Discard))
            .await,
        Ok(None)
    );
    service
        .start_turn(session_id, "Anything else?".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    // Two events, one item, in the wake turn's request.
    assert_eq!(fragments(&requests[0].body), 1);
    let woken = request_text(&requests[0].body);
    assert!(woken.contains("first answer") && woken.contains("second answer"));
    // The next request carries it as history, once.
    assert_eq!(fragments(&requests[1].body), 1);
    // The wake turn opens with the events and the world state, and no user
    // message.
    let items = transcript(&service, session_id);
    assert!(
        matches!(
            &items[..],
            [
                ConversationItem::TurnStart { .. },
                ConversationItem::WorldStateUpdate { .. },
                ConversationItem::WakeEvents { events },
                ConversationItem::AssistantMessage { .. },
                ConversationItem::TurnStart { .. },
                ..
            ] if events.len() == 2
        ),
        "{items:#?}"
    );
    let export = service.export_session_markdown(session_id).unwrap();
    assert!(
        export.contains(
            "## Background updates\n\n- Sub-agent `agent_41` (agent id 41) completed: first answer\n- Sub-agent `agent_42` (agent id 42) completed: second answer\n"
        ),
        "{export}"
    );
    // `/context` counts the item with the calls and their outputs.
    let context = service.session_context(session_id).await.unwrap();
    let AgentContextRequest::Sent(sent) = context.request else {
        panic!("{:?}", context.request);
    };
    assert!(sent.composition.tool_calls_tokens > 0, "{sent:#?}");
    assert!(!sent
        .composition
        .sections
        .iter()
        .any(|section| section.kind == "background_events"));
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// An event queued while the turn's response streams reaches its next
/// step, and the turn does not end before it does.
#[tokio::test(flavor = "multi_thread")]
async fn an_event_queued_during_a_turn_reaches_its_next_step() {
    let held = String::from_utf8(assistant_events("Working."))
        .unwrap()
        .replace(
            "data: {\"type\":\"response.completed\"",
            &format!("{HOLD}data: {{\"type\":\"response.completed\""),
        )
        .into_bytes();
    let server = FakeServer::start(vec![
        (held, Duration::ZERO),
        (assistant_events("Read the update."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("wake-running", &server.base_url);
    let service = Arc::new(service);
    let session_id = session_with_events(&service, Vec::new()).await;
    let turn = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .start_turn(session_id, "Start.".to_string().into())
                .await
                .unwrap()
        }
    });
    tokio::time::timeout(TEST_WAIT, async {
        while server.requests().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the turn's request was sent");
    assert_eq!(
        queue_wake_event(&service, session_id, completed(41, "late answer")).await,
        Ok(AgentWakeQueued::Delivering)
    );
    server.hold.notify_one();

    let outcome = turn.await.unwrap();
    assert_eq!(outcome.final_answer, "Read the update.");
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(fragments(&requests[0].body), 0);
    assert_eq!(fragments(&requests[1].body), 1);
    assert!(request_text(&requests[1].body).contains("late answer"));
    assert_eq!(service.snapshot(session_id).unwrap().pending_wake_events, 0);
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The queue is durable until delivered, and the delivered item is
/// history: both survive a cold reopen.
#[tokio::test(flavor = "multi_thread")]
async fn the_queue_and_the_item_survive_a_cold_reopen() {
    let server = FakeServer::start(vec![
        (assistant_events("Noted the first."), Duration::ZERO),
        (assistant_events("Noted the second."), Duration::ZERO),
        (assistant_events("Still here."), Duration::ZERO),
    ])
    .await;
    let (service, paths, config) = service_with_config("wake-reopen", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    let session_id = session_with_events(&service, vec![completed(41, "before the restart")]).await;
    run_wake_turn(&service, session_id).await;
    queue_wake_event(
        &service,
        session_id,
        completed(42, "queued before the restart"),
    )
    .await
    .unwrap();
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    drop(service);

    let service = reopen_service(config, test_host(&paths), &paths);
    service.core.activate_session(session_id).unwrap();
    assert_eq!(service.snapshot(session_id).unwrap().pending_wake_events, 1);
    set_wake_policy(&service, session_id, AgentWakePolicy::On)
        .await
        .unwrap();
    service
        .start_turn(session_id, "What changed?".to_string().into())
        .await
        .unwrap();
    service
        .start_turn(session_id, "And now?".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    let after_restart = request_text(&requests[1].body);
    assert_eq!(fragments(&requests[1].body), 2);
    assert!(after_restart.contains("before the restart"));
    assert!(after_restart.contains("queued before the restart"));
    // Each delivered once, then carried as history.
    assert_eq!(fragments(&requests[2].body), 2);
    let delivered = transcript(&service, session_id)
        .into_iter()
        .filter(|item| matches!(item, ConversationItem::WakeEvents { .. }))
        .count();
    assert_eq!(delivered, 2);
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A `!command`'s turn samples no model, so it delivers nothing: the
/// events wait for the next turn that does.
#[tokio::test(flavor = "multi_thread")]
async fn a_shell_commands_turn_leaves_the_events_queued() {
    let server = FakeServer::start(vec![(assistant_events("Read it."), Duration::ZERO)]).await;
    let (service, paths) = configured_service("wake-shell", &server.base_url);
    let session_id = session_with_events(&service, vec![completed(41, "done meanwhile")]).await;
    let command = service
        .submit_user_shell_command_with_sink(
            session_id,
            "echo wake-shell".to_string(),
            Arc::new(Discard),
        )
        .await
        .unwrap();
    wait_for_end(&service, &command).await;
    assert!(!transcript(&service, session_id)
        .iter()
        .any(|item| matches!(item, ConversationItem::WakeEvents { .. })));
    assert_eq!(service.snapshot(session_id).unwrap().pending_wake_events, 1);

    service
        .start_turn(session_id, "What ran?".to_string().into())
        .await
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(fragments(&requests[0].body), 1);
    assert!(request_text(&requests[0].body).contains("done meanwhile"));
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A standard compaction summarizes the item with the rest: the summary
/// request reads it, and the next turn's window no longer carries it.
#[tokio::test(flavor = "multi_thread")]
async fn a_compaction_summarizes_the_item() {
    let server = FakeServer::start(vec![
        (assistant_events("Noted."), Duration::ZERO),
        (
            assistant_events("A sub-agent reported its answer."),
            Duration::ZERO,
        ),
        (assistant_events("Still here."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("wake-compaction", &server.base_url);
    let session_id = session_with_events(&service, vec![completed(41, "to summarize")]).await;
    run_wake_turn(&service, session_id).await;
    let before = service.snapshot(session_id).unwrap();
    service
        .compact_session(session_id, before.generation, before.revision)
        .await
        .unwrap();
    service
        .start_turn(session_id, "Are you there?".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(fragments(&requests[1].body), 1);
    assert_eq!(fragments(&requests[2].body), 0);
    assert!(request_text(&requests[2].body).contains("A sub-agent reported its answer."));
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
