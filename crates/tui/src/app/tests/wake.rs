//! A session woken by its sub-agents, end to end over the standalone
//! Agent, the parent on the scripted model and its sub-agent on a second
//! Provider's (each held where a test needs an order): the session that
//! ended its turn waits, saying so; the agent's end starts the woken turn
//! by itself, opened by the background updates, in either presentation, and
//! in the main conversation while another one is shown; a message typed
//! while it waits is the person's turn, after which it waits again; another
//! session's wake starts nothing here.

use std::time::Duration;

use hanafubuki_agent::test_support::{
    add_test_provider, assistant_events, tool_events, FakeServer, HOLD,
};
use hanafubuki_agent::{AgentBackgroundEvent, AgentSessionId};
use ratatui::crossterm::event::KeyCode;

use super::Harness;
use crate::app::WorkspaceEvent;
use crate::app_event::AppEvent;
use crate::transcript_mode::TranscriptMode;

/// The first line of a `<background_events>` fragment.
const UPDATES: &str = "Updates on work you started in the background";
const WAITING: &str = "Waiting for 1 agent — resumes by itself";

/// The sub-agents' model, on a runtime of its own.
struct Children {
    _runtime: tokio::runtime::Runtime,
    server: FakeServer,
}

impl Children {
    fn start(responses: Vec<Vec<u8>>) -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = runtime.block_on(FakeServer::start(
            responses
                .into_iter()
                .map(|body| (body, Duration::ZERO))
                .collect(),
        ));
        Self {
            _runtime: runtime,
            server,
        }
    }
}

/// `events`, held until the server's hold is released once.
fn held(events: Vec<u8>) -> Vec<u8> {
    [HOLD.as_bytes(), &events].concat()
}

/// The parent's step that spawns `counter` on the second Provider.
fn spawn_counter() -> Vec<u8> {
    let arguments = serde_json::json!({
        "task_name": "counter",
        "message": "Count the files.",
        "fork_turns": "none",
        "provider": "provider-2",
        "model": "fixture-model",
    });
    tool_events("spawn_agent", &arguments.to_string())
}

/// The interface in `mode`, its sessions on `parent`'s answers and its
/// sub-agents on `children`.
fn harness(mode: TranscriptMode, parent: Vec<Vec<u8>>, children: &Children) -> Harness {
    let base_url = children.server.base_url.clone();
    Harness::start_configured(
        parent
            .into_iter()
            .map(|body| (body, Duration::ZERO))
            .collect(),
        true,
        mode,
        move |config| add_test_provider(config, "provider-2", "Second", &base_url),
    )
}

/// The parent delegates and ends its turn; the interface says it waits.
/// `later` answers its next requests.
fn delegated(mode: TranscriptMode, children: &Children, later: Vec<Vec<u8>>) -> Harness {
    let mut parent = vec![spawn_counter(), assistant_events("The counter is on it.")];
    parent.extend(later);
    let mut harness = harness(mode, parent, children);
    harness.type_and_submit("Delegate the count.");
    harness.pump_until_screen_shows("The counter is on it.");
    harness.pump_until_turn_ends();
    harness.pump_until_screen_shows(WAITING);
    harness
}

fn the_session_waits_and_resumes_by_itself(mode: TranscriptMode) -> Harness {
    let children = Children::start(vec![held(assistant_events("Twelve files."))]);
    let mut harness = delegated(
        mode,
        &children,
        vec![held(assistant_events("There are twelve."))],
    );

    children.server.hold.notify_one();
    harness.pump_until("the woken turn's updates", |harness| {
        harness.transcript().contains("• Background updates")
    });
    harness.server.hold.notify_one();
    harness.pump_until_screen_shows("There are twelve.");
    harness.pump_until_turn_ends();

    let transcript = harness.transcript();
    let updates = transcript.find("• Background updates").unwrap();
    assert!(
        transcript[updates..].contains("Sub-agent `counter`"),
        "{transcript}"
    );
    assert!(updates < transcript.find("There are twelve.").unwrap());
    assert!(!harness.screen().contains("Waiting for"));
    let requests = harness.server.requests();
    assert_eq!(requests.len(), 3);
    assert!(String::from_utf8_lossy(&requests[2].body).contains(UPDATES));
    harness
}

#[test]
fn a_session_waits_for_its_agent_and_resumes_by_itself_in_scrollback() {
    the_session_waits_and_resumes_by_itself(TranscriptMode::Terminal);
}

#[test]
fn a_session_waits_for_its_agent_and_resumes_by_itself_on_the_alternate_screen() {
    let mut harness = the_session_waits_and_resumes_by_itself(TranscriptMode::Owned);
    // Another session's wake is its own: nothing starts here.
    let requests = harness.server.requests().len();
    harness.app.handle_event(
        &mut harness.tui,
        AppEvent::AgentBackground(AgentBackgroundEvent::Woken {
            session_id: AgentSessionId::new(999),
        }),
    );
    harness.pump_until_turn_ends();
    assert!(!harness.app.chat_widget.is_task_running());
    assert_eq!(harness.server.requests().len(), requests);
}

#[test]
fn the_waiting_line_takes_the_status_indicators_place() {
    let children = Children::start(vec![held(assistant_events("Twelve files."))]);
    let harness = delegated(TranscriptMode::Owned, &children, Vec::new());
    insta::assert_snapshot!(harness.bottom_rows(7));
}

/// The main conversation is woken while a sub-agent is shown in its place:
/// its woken turn runs parked, and is there when it is shown again.
#[test]
fn a_parked_main_conversation_is_woken() {
    let children = Children::start(vec![held(assistant_events("Twelve files."))]);
    let mut harness = delegated(
        TranscriptMode::Terminal,
        &children,
        vec![held(assistant_events("There are twelve."))],
    );
    let parent = harness.app.bridge.session_id().unwrap();
    let [child] = harness.subagents_of(parent)[..] else {
        panic!("one subagent");
    };
    harness.app.handle_event(
        &mut harness.tui,
        AppEvent::Workspace(WorkspaceEvent::OpenChild(child)),
    );
    harness.pump_until_screen_shows("Subagent counter");

    children.server.hold.notify_one();
    harness.pump_until("the parked conversation's woken turn", |harness| {
        harness.server.requests().len() == 3
    });
    harness.server.hold.notify_one();
    let agent = harness.runtime.slot().agent().unwrap();
    harness.pump_until("the woken turn ends", |_| {
        agent
            .snapshot(parent)
            .unwrap()
            .turn
            .is_some_and(|turn| turn.phase.is_terminal() && turn.id.value() > 1)
    });
    // The sub-agent shown has seen its own turn end: Esc returns.
    harness.pump_until_turn_ends();
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Main conversation");
    harness.pump_until("the woken turn's answer", |harness| {
        harness.transcript().contains("There are twelve.")
    });
    assert!(harness.transcript().contains("• Background updates"));
}

/// A message typed while the session waits is the person's turn, as at
/// any time: it is answered, the session waits again, and its agent's end
/// still wakes it.
#[test]
fn a_message_typed_while_waiting_is_the_persons_turn() {
    let children = Children::start(vec![held(assistant_events("Twelve files."))]);
    let mut harness = delegated(
        TranscriptMode::Terminal,
        &children,
        vec![
            assistant_events("Still counting."),
            held(assistant_events("There are twelve.")),
        ],
    );

    harness.type_and_submit("How is it going?");
    harness.pump_until_screen_shows("Still counting.");
    harness.pump_until_turn_ends();
    harness.pump_until_screen_shows(WAITING);
    children.server.hold.notify_one();
    harness.pump_until("the woken turn's updates", |harness| {
        harness.transcript().contains("• Background updates")
    });
    harness.server.hold.notify_one();
    harness.pump_until_screen_shows("There are twelve.");
    harness.pump_until_turn_ends();

    let requests = harness.server.requests();
    assert_eq!(requests.len(), 4);
    let asked = String::from_utf8_lossy(&requests[2].body).to_string();
    assert!(asked.contains("How is it going?") && !asked.contains(UPDATES));
    assert!(String::from_utf8_lossy(&requests[3].body).contains(UPDATES));
}
