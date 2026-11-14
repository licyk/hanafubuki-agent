//! What a parent hears of its agents through its wake queue, over two
//! scripted models (the parent on one, its agents on the other, so a test
//! can hold either): a final status is queued on it durably and
//! `wait_agent` takes it once (after a restart too); a follow-up task
//! withdraws the status its previous turn left; a parent that ended its
//! turn is woken once and its woken turn reads the answer; an agent that
//! ends while its parent's turn ends reaches that turn, never a woken one;
//! two ends make one woken turn; an agent's messages wake it, at most 16
//! unread; a status a step delivered is not waited for again. Pauses, the
//! option and the sessions never woken are `pause.rs`'s.

use hanafubuki_agent_kernel::{
    AgentEvent, AgentEventSink, AgentSubagentWake, AgentTurnAccepted, AgentWakePolicy,
    AgentWakeSource,
};

use super::*;
use crate::test_support::{
    add_test_provider, queue_wake_event, reopen_service, reopen_service_waking, set_wake_policy,
    test_paths, tool_events, RecordedBackgroundEvents, HOLD,
};
use crate::{AgentConfig, AgentTurnRequest, AgentWakeState};

/// Waits until `session_id` has `count` wake events queued.
pub(super) async fn queued(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    count: usize,
) {
    wait_until(&format!("{count} wake events are queued"), || {
        service.snapshot(session_id).unwrap().pending_wake_events == count
    })
    .await;
}

/// An Agent whose sessions run on `parent` (`provider-1`) and whose agents
/// may run on `children` (`provider-2`), after `configure`, telling the
/// returned recorder whom to wake; it submits nothing. No title is asked of
/// the models: the person's turns would.
pub(super) async fn waking(
    name: &str,
    parent: &FakeServer,
    children: &FakeServer,
    configure: impl FnOnce(&AgentConfig),
) -> (
    AgentApplicationService,
    TestPaths,
    AgentConfig,
    RecordedBackgroundEvents,
) {
    let paths = test_paths(name);
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, &parent.base_url, None);
    add_test_provider(&config, "provider-2", "Second", &children.base_url);
    config.set_plugin_disabled("titles", true).unwrap();
    configure(&config);
    let woken = RecordedBackgroundEvents::default();
    let service = reopen_service_waking(
        config.clone(),
        test_host(&paths),
        &paths,
        Arc::new(woken.clone()),
    );
    service.start_plugins().await;
    (service, paths, config, woken)
}

/// The parent's step that spawns `name` on the children's model with
/// `task`.
pub(super) fn spawn_on_second(name: &str, task: &str) -> Vec<u8> {
    let arguments = json!({
        "task_name": name,
        "message": task,
        "fork_turns": "none",
        "provider": "provider-2",
        "model": "fixture-model",
    });
    tool_events("spawn_agent", &arguments.to_string())
}

/// `events`, held until the server's hold is released once.
pub(super) fn held(events: Vec<u8>) -> Vec<u8> {
    [HOLD.as_bytes(), &events].concat()
}

pub(super) struct Discard;

impl AgentEventSink for Discard {
    fn emit(&self, _event: AgentEvent) {}
}

/// Submits a person's message to `session_id`, as an interface does.
pub(super) async fn person(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    text: &str,
) -> AgentTurnAccepted {
    service
        .submit_turn_with_sink(
            session_id,
            AgentTurnRequest {
                text: text.to_string(),
                images: Vec::new(),
                skills: Vec::new(),
                host: Default::default(),
            },
            Arc::new(Discard),
        )
        .await
        .unwrap()
}

/// Waits until `turn` ended.
pub(super) async fn ended(service: &AgentApplicationService, turn: AgentTurnAccepted) {
    wait_until("the turn ends", || {
        service
            .events_since(turn.session_id, None)
            .unwrap()
            .events
            .iter()
            .any(|event| event.turn_id == Some(turn.turn_id) && event.kind.is_terminal())
    })
    .await;
}

/// Waits until `parent` has spawned an agent and none of its agents runs:
/// each has queued what its parent hears of its end.
pub(super) async fn agents_done(subagents: &Arc<SubagentService>, parent: AgentSessionId) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let agents = subagents.agents(parent).await.unwrap();
        if !agents.is_empty()
            && agents
                .iter()
                .all(|agent| agent.status != AgentRunStatus::Running)
        {
            return;
        }
        assert!(Instant::now() < deadline, "the agents did not end");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Waits until the dispatcher has looked at every signal sent so far: it
/// looks at the sessions signalled together in the order of their ids, so a
/// session created and woken now is woken after the earlier ones were
/// looked at.
pub(super) async fn settled(service: &AgentApplicationService, woken: &RecordedBackgroundEvents) {
    let probe = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    set_wake_policy(service, probe, AgentWakePolicy::On)
        .await
        .unwrap();
    queue_wake_event(service, probe, message(probe, "probe"))
        .await
        .unwrap();
    wait_until("the dispatcher reaches the probe", || {
        woken.woken(probe) >= 1
    })
    .await;
}

fn message(session_id: AgentSessionId, text: &str) -> AgentWakeSource {
    AgentWakeSource::Subagent {
        session_id,
        name: "probe".to_string(),
        event: AgentSubagentWake::Message {
            text: text.to_string(),
        },
    }
}

/// The first line of a `<background_events>` fragment: the guidance names
/// the tag too.
pub(super) const UPDATES: &str = "Updates on work you started in the background";

fn text(server: &FakeServer, index: usize) -> String {
    String::from_utf8(server.requests()[index].body.clone()).unwrap()
}

/// An agent's final status is queued on its parent, which takes it with
/// its next wait, after a restart too, and only once.
#[tokio::test(flavor = "multi_thread")]
async fn a_final_status_waits_in_the_parents_queue_across_a_restart() {
    let server = FakeServer::start(vec![(assistant_events("Twelve files."), Duration::ZERO)]).await;
    let (service, paths, config) = service_with_host("subagents-queued", test_host, |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    service.start_plugins().await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let child = subagents
        .spawn(parent, spawn("counter", "Count the files."))
        .await
        .unwrap()
        .id;
    queued(&service, parent, 1).await;
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let reopened = reopen_service(config, test_host(&paths), &paths);
    reopened.start_plugins().await;
    let subagents = super::subagents(&reopened).await;
    let cancellation = CancellationToken::default();
    assert_eq!(
        subagents
            .wait(parent, NO_WAIT, &cancellation)
            .await
            .unwrap(),
        WaitOutcome::Updates(vec![AgentUpdate {
            name: "counter".to_string(),
            id: child,
            status: AgentRunStatus::Completed(Some("Twelve files.".to_string())),
        }])
    );
    assert_eq!(reopened.snapshot(parent).unwrap().pending_wake_events, 0);
    assert_eq!(
        subagents
            .wait(parent, NO_WAIT, &cancellation)
            .await
            .unwrap(),
        WaitOutcome::Idle,
        "a status is reported once"
    );
    finish(reopened, paths);
}

/// A follow-up task given to an agent whose last status was not waited for
/// withdraws it: the next wait reports the follow-up's end, as it did when
/// the status was kept in memory.
#[tokio::test(flavor = "multi_thread")]
async fn a_follow_up_withdraws_the_status_its_previous_turn_left() {
    let server = FakeServer::start(vec![
        (assistant_events("First answer."), Duration::ZERO),
        (assistant_events("Second answer."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = started("subagents-withdrawn", &server).await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    subagents
        .spawn(parent, spawn("worker", "Answer once."))
        .await
        .unwrap();
    queued(&service, parent, 1).await;

    assert_eq!(
        subagents
            .followup_task(parent, "worker", "Answer again.".into())
            .await
            .unwrap(),
        Delivery::Started
    );
    let waited = subagents
        .wait(parent, NO_WAIT, &CancellationToken::default())
        .await
        .unwrap();
    assert!(
        matches!(
            &waited,
            WaitOutcome::Updates(updates)
                if updates.len() == 1
                    && updates[0].status
                        == AgentRunStatus::Completed(Some("Second answer.".to_string()))
        ),
        "{waited:?}"
    );
    finish(service, paths);
}

/// The round trip: the parent spawns an agent and ends its turn without
/// waiting, holding nothing while the agent works; the agent's end wakes
/// it once; the woken turn opens with the agent's answer, and answers.
#[tokio::test(flavor = "multi_thread")]
async fn a_parent_that_ended_its_turn_is_woken_once_with_its_agents_answer() {
    let parent = FakeServer::start(vec![
        (
            spawn_on_second("counter", "Count the files."),
            Duration::ZERO,
        ),
        (assistant_events("The counter is on it."), Duration::ZERO),
        (assistant_events("There are twelve."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(vec![(
        held(assistant_events("Twelve files.")),
        Duration::ZERO,
    )])
    .await;
    let (service, paths, _, woken) = waking("wake-round-trip", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    let first = person(&service, root, "Delegate the count.").await;
    ended(&service, first).await;
    let guided = text(&parent, 0);
    assert!(
        guided.contains("You do not have to wait for sub-agents."),
        "{guided}"
    );
    assert!(!guided.contains("wait for them before yielding"));
    let status = service.background_status(root).unwrap();
    assert_eq!(
        (status.work.agents, status.pending_wake_events, status.wake),
        (1, 0, AgentWakeState::On)
    );
    assert!(service
        .snapshot(root)
        .unwrap()
        .turn
        .unwrap()
        .phase
        .is_terminal());
    // The turn's end was looked at, with nothing queued yet.
    settled(&service, &woken).await;
    assert_eq!(woken.woken(root), 0);

    children.hold.notify_one();
    wait_until("the parent is woken", || woken.woken(root) >= 1).await;
    queued(&service, root, 1).await;
    // A host may request another check before accepting the first hint.
    // Both notifications still refer to the same queued completion.
    let notified = woken.woken(root);
    service.resume_background_wakes(root).unwrap();
    wait_until("the parent is notified again", || {
        woken.woken(root) > notified
    })
    .await;
    let wake = service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .expect("the agent's end is queued");
    assert_eq!(
        wake.events
            .iter()
            .map(|event| event.source.summary())
            .collect::<Vec<_>>(),
        vec![format!(
            "Sub-agent `counter` (agent id {}) completed: Twelve files.",
            subagents(&service).await.agents(root).await.unwrap()[0].id
        )]
    );
    ended(&service, wake.turn).await;

    let woken_request = text(&parent, 2);
    assert_eq!(woken_request.matches(UPDATES).count(), 1);
    assert!(woken_request.contains("Twelve files."), "{woken_request}");
    assert_eq!(parent.requests().len(), 3);
    assert_eq!(
        service.subagents(root).await.unwrap()[0].last_answer,
        Some("Twelve files.".to_string())
    );
    settled(&service, &woken).await;
    assert!(
        service
            .submit_wake_turn_with_sink(root, Arc::new(Discard))
            .await
            .unwrap()
            .is_none(),
        "the repeated hints admit only one turn"
    );
    let status = service.background_status(root).unwrap();
    assert_eq!((status.work.agents, status.pending_wake_events), (0, 0));
    finish(service, paths);
}

/// The race at a turn's end: an agent that ends while its parent's turn is
/// finishing (its last response held) reaches that turn, which takes one
/// more step to read it; nothing is woken, and it is read once.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_ending_while_its_parents_turn_ends_reaches_that_turn() {
    let parent = FakeServer::start(vec![
        (
            spawn_on_second("counter", "Count the files."),
            Duration::ZERO,
        ),
        (held(assistant_events("Done for now.")), Duration::ZERO),
        (
            assistant_events("The counter found twelve."),
            Duration::ZERO,
        ),
    ])
    .await;
    let children = FakeServer::start(vec![(
        held(assistant_events("Twelve files.")),
        Duration::ZERO,
    )])
    .await;
    let (service, paths, _, woken) = waking("wake-race", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let subagents = subagents(&service).await;

    // The agent ends while the parent's last response is held.
    let turn = person(&service, root, "Delegate the count.").await;
    wait_until("the parent's last step samples", || {
        parent.requests().len() == 2
    })
    .await;
    children.hold.notify_one();
    agents_done(&subagents, root).await;
    queued(&service, root, 1).await;
    assert!(!service
        .snapshot(root)
        .unwrap()
        .turn
        .unwrap()
        .phase
        .is_terminal());
    parent.hold.notify_one();
    ended(&service, turn).await;

    assert_eq!(parent.requests().len(), 3);
    let last = text(&parent, 2);
    assert_eq!(last.matches(UPDATES).count(), 1);
    assert!(last.contains("Twelve files.") && last.contains("Done for now."));
    settled(&service, &woken).await;
    assert_eq!(
        woken.woken(root),
        0,
        "a running turn reads it; nothing is woken"
    );
    assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 0);
    assert_eq!(
        service
            .submit_wake_turn_with_sink(root, Arc::new(Discard))
            .await
            .unwrap(),
        None
    );
    finish(service, paths);
}

/// Two agents that end together make one woken turn, which reads both.
#[tokio::test(flavor = "multi_thread")]
async fn two_agents_ending_together_make_one_woken_turn() {
    let spawns = |name: &str| {
        json!({
            "task_name": name,
            "message": format!("Count the {name} files."),
            "fork_turns": "none",
            "provider": "provider-2",
            "model": "fixture-model",
        })
        .to_string()
    };
    let parent = FakeServer::start(vec![
        (
            tool_calls_events(&[
                ("call-a", "spawn_agent", &spawns("rust")),
                ("call-b", "spawn_agent", &spawns("docs")),
            ]),
            Duration::ZERO,
        ),
        (assistant_events("Both are counting."), Duration::ZERO),
        (assistant_events("Both reported."), Duration::ZERO),
    ])
    .await;
    // The second answer waits behind the first, which is held.
    let children = FakeServer::start(vec![
        (held(assistant_events("Seven files.")), Duration::ZERO),
        (assistant_events("Five files."), Duration::ZERO),
    ])
    .await;
    let (service, paths, _, woken) = waking("wake-together", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    let first = person(&service, root, "Count both.").await;
    ended(&service, first).await;
    children.hold.notify_one();
    queued(&service, root, 2).await;
    wait_until("the parent is woken", || woken.woken(root) >= 1).await;
    let wake = service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wake.events.len(), 2);
    ended(&service, wake.turn).await;

    let read = text(&parent, 2);
    assert_eq!(read.matches(UPDATES).count(), 1);
    assert!(read.contains("Seven files.") && read.contains("Five files."));
    assert_eq!(
        service
            .submit_wake_turn_with_sink(root, Arc::new(Discard))
            .await
            .unwrap(),
        None,
        "nothing is left for a second woken turn"
    );
    assert!(woken.woken(root) >= 1);
    finish(service, paths);
}

/// An agent's `send_message` to `parent` queues a message on its parent,
/// which wakes it; at most 16 unread are queued, and the agent is told the
/// 17th was refused. The woken turn reads the messages and the agent's end.
#[tokio::test(flavor = "multi_thread")]
async fn an_agents_messages_wake_its_parent_at_most_16_unread() {
    let messages = (1..=17)
        .map(|index| {
            (
                format!("call-{index}"),
                json!({ "target": "parent", "message": format!("Note {index}.") }).to_string(),
            )
        })
        .collect::<Vec<_>>();
    // A step makes at most 8 calls.
    let calls = |range: std::ops::Range<usize>| {
        tool_calls_events(
            &messages[range]
                .iter()
                .map(|(id, arguments)| (id.as_str(), "send_message", arguments.as_str()))
                .collect::<Vec<_>>(),
        )
    };
    let parent = FakeServer::start(vec![
        (
            spawn_on_second("counter", "Count, and keep me posted."),
            Duration::ZERO,
        ),
        (assistant_events("The counter is on it."), Duration::ZERO),
        (assistant_events("Read the notes."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(vec![
        (held(calls(0..8)), Duration::ZERO),
        (calls(8..16), Duration::ZERO),
        (calls(16..17), Duration::ZERO),
        (assistant_events("Counted."), Duration::ZERO),
    ])
    .await;
    let (service, paths, _, woken) = waking("wake-messages", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    let first = person(&service, root, "Count with an agent.").await;
    ended(&service, first).await;
    children.hold.notify_one();
    queued(&service, root, 17).await;
    wait_until("the parent is woken", || woken.woken(root) >= 1).await;

    let told = text(&children, 3);
    assert_eq!(told.matches("queued for its next turn").count(), 16);
    assert!(
        told.contains("your parent has 16 unread messages; wait until it reads them"),
        "{told}"
    );
    let wake = service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .unwrap();
    ended(&service, wake.turn).await;
    let read = text(&parent, 2);
    assert_eq!(read.matches("Message from sub-agent `counter`").count(), 16);
    assert!(read.contains("Note 16.") && !read.contains("Note 17."));
    assert!(read.contains("Counted."));
    finish(service, paths);
}

/// A status a step delivered is not waited for again: the step after the
/// agent's end reads it as an update, and the wait that follows finds no
/// agent working and returns nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_status_a_step_delivered_is_not_waited_for_again() {
    let parent = FakeServer::start(vec![
        (
            spawn_on_second("counter", "Count the files."),
            Duration::ZERO,
        ),
        (held(tool_events("list_agents", "{}")), Duration::ZERO),
        (
            tool_events("wait_agent", r#"{"timeout_ms":20000}"#),
            Duration::ZERO,
        ),
        (assistant_events("All told."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(vec![(
        held(assistant_events("Twelve files.")),
        Duration::ZERO,
    )])
    .await;
    let (service, paths, _, _) = waking("wake-then-wait", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let subagents = subagents(&service).await;

    // The agent ends while the parent's second step samples.
    let turn = person(&service, root, "Delegate the count.").await;
    wait_until("the parent's second step samples", || {
        parent.requests().len() == 2
    })
    .await;
    children.hold.notify_one();
    agents_done(&subagents, root).await;
    parent.hold.notify_one();
    ended(&service, turn).await;

    let delivered = text(&parent, 2);
    assert_eq!(delivered.matches(UPDATES).count(), 1);
    assert!(delivered.contains("Twelve files."));
    let waited = text(&parent, 3);
    assert!(
        waited.contains("None of your agents is still working."),
        "{waited}"
    );
    assert_eq!(
        waited.matches("Twelve files.").count(),
        1,
        "only the update in history carries the answer"
    );
    finish(service, paths);
}

/// A status queued before a restart is kept, nothing is woken while the
/// Agent starts, and the person's next message reads it.
#[tokio::test(flavor = "multi_thread")]
async fn a_queued_status_is_read_with_the_next_message_after_a_restart() {
    let parent = FakeServer::start(vec![
        (
            spawn_on_second("counter", "Count the files."),
            Duration::ZERO,
        ),
        (assistant_events("The counter is on it."), Duration::ZERO),
        (assistant_events("It found twelve."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(vec![(
        held(assistant_events("Twelve files.")),
        Duration::ZERO,
    )])
    .await;
    let (service, paths, config, woken) = waking("wake-restart", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let first = person(&service, root, "Delegate the count.").await;
    ended(&service, first).await;
    children.hold.notify_one();
    wait_until("the parent is woken", || woken.woken(root) >= 1).await;
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let woken = RecordedBackgroundEvents::default();
    let reopened =
        reopen_service_waking(config, test_host(&paths), &paths, Arc::new(woken.clone()));
    reopened.start_plugins().await;
    settled(&reopened, &woken).await;
    assert_eq!(woken.woken(root), 0, "nothing is woken at start");
    assert_eq!(
        reopened
            .background_status(root)
            .unwrap()
            .pending_wake_events,
        1
    );
    let next = person(&reopened, root, "What did it find?").await;
    ended(&reopened, next).await;

    let read = text(&parent, 2);
    let events = read.find(UPDATES).expect("the update");
    let message = read.find("What did it find?").expect("the message");
    assert!(events < message, "the update comes before the message");
    assert!(read.contains("Twelve files."));
    assert_eq!(read.matches(UPDATES).count(), 1);
    assert_eq!(parent.requests().len(), 3);
    assert_eq!(reopened.snapshot(root).unwrap().pending_wake_events, 0);
    finish(reopened, paths);
}
