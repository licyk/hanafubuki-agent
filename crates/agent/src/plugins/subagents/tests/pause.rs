//! When a parent is not woken for its agents: the person interrupted its
//! turn, or its tree ran past the runaway bound (both until the person's
//! next message, which reads what queued); `wakeOnBackgroundTasks` is off,
//! from the start or turned off while an agent works; it is archived; it is
//! a side conversation. A woken turn in Plan mode, and a goal that lets the
//! turn end while an agent works and goes on in the woken turn.

use hanafubuki_agent_kernel::{AgentContinuationHookOutcome, AgentSubagentWake, AgentWakePolicy};

use super::wake::{
    agents_done, ended, held, person, queued, settled, spawn_on_second, waking, Discard, UPDATES,
};
use super::*;
use crate::test_support::{queue_wake_event, set_wake_policy, tool_events};
use crate::{
    AgentGoalBudgets, AgentSettingsPatch, AgentWakePause, AgentWakeSource, AgentWakeState,
};

fn text(server: &FakeServer, index: usize) -> String {
    String::from_utf8(server.requests()[index].body.clone()).unwrap()
}

fn completed(name: &str, answer: &str) -> AgentWakeSource {
    AgentWakeSource::Subagent {
        session_id: AgentSessionId::new(4_000),
        name: name.to_string(),
        event: AgentSubagentWake::Completed {
            answer: Some(answer.to_string()),
            truncated: false,
        },
    }
}

fn wake_on_background_tasks(enabled: bool) -> impl FnOnce(&crate::AgentConfig) {
    move |config| {
        config
            .write_settings(AgentSettingsPatch {
                wake_on_background_tasks: Some(enabled),
                ..AgentSettingsPatch::default()
            })
            .unwrap();
    }
}

/// The person's interrupt pauses the parent's wakes: its agent's end is
/// queued and wakes nothing; the person's next message reads it and the
/// wakes resume.
#[tokio::test(flavor = "multi_thread")]
async fn the_persons_interrupt_pauses_wakes_until_their_next_message() {
    let parent = FakeServer::start(vec![
        (
            spawn_on_second("counter", "Count the files."),
            Duration::ZERO,
        ),
        (held(assistant_events("Never read.")), Duration::ZERO),
        (assistant_events("It found twelve."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(vec![(
        held(assistant_events("Twelve files.")),
        Duration::ZERO,
    )])
    .await;
    let (service, paths, _, woken) = waking("wake-paused", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let subagents = subagents(&service).await;

    let turn = person(&service, root, "Delegate the count.").await;
    wait_until("the parent's second step samples", || {
        parent.requests().len() == 2
    })
    .await;
    service
        .cancel_turn(root, turn.turn_id, turn.generation)
        .await
        .unwrap();
    ended(&service, turn).await;
    parent.hold.notify_one();
    children.hold.notify_one();
    agents_done(&subagents, root).await;
    queued(&service, root, 1).await;
    settled(&service, &woken).await;

    assert_eq!(woken.woken(root), 0, "paused: nothing is woken");
    assert_eq!(
        service.background_status(root).unwrap().wake,
        AgentWakeState::Paused {
            reason: AgentWakePause::PersonInterrupted
        }
    );
    let next = person(&service, root, "What did it find?").await;
    ended(&service, next).await;
    let read = text(&parent, 2);
    assert!(read.find(UPDATES).unwrap() < read.find("What did it find?").unwrap());
    assert!(read.contains("Twelve files."));
    assert_eq!(
        service.background_status(root).unwrap().wake,
        AgentWakeState::On
    );
    finish(service, paths);
}

/// Past the runaway bound (lowered to 2 here), a tree's wakes pause until
/// the person's next message; a host's own submission still runs.
#[tokio::test(flavor = "multi_thread")]
async fn wakes_pause_past_the_runaway_bound() {
    let parent = FakeServer::start(vec![
        (assistant_events("First."), Duration::ZERO),
        (assistant_events("Second."), Duration::ZERO),
        (assistant_events("Third."), Duration::ZERO),
        (assistant_events("Read."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(Vec::new()).await;
    let (service, paths, _, woken) = waking("wake-bound", &parent, &children, |_| {}).await;
    service.wakes.pauses.set_bound(2);
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    set_wake_policy(&service, root, AgentWakePolicy::On)
        .await
        .unwrap();

    // A turn's end and the event after it may each wake the parent: the
    // host's submission is what counts.
    for answer in ["one", "two"] {
        let before = woken.woken(root);
        queue_wake_event(&service, root, completed("counter", answer))
            .await
            .unwrap();
        wait_until("the parent is woken", || woken.woken(root) > before).await;
        let wake = service
            .submit_wake_turn_with_sink(root, Arc::new(Discard))
            .await
            .unwrap()
            .unwrap();
        ended(&service, wake.turn).await;
    }
    settled(&service, &woken).await;
    let before = woken.woken(root);
    queue_wake_event(&service, root, completed("counter", "three"))
        .await
        .unwrap();
    settled(&service, &woken).await;

    assert_eq!(woken.woken(root), before, "the third is not woken");
    assert_eq!(
        service.background_status(root).unwrap().wake,
        AgentWakeState::Paused {
            reason: AgentWakePause::RunawayBound
        }
    );
    let explicit = service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .expect("a host's submission runs while paused");
    ended(&service, explicit.turn).await;
    queue_wake_event(&service, root, completed("counter", "four"))
        .await
        .unwrap();
    let next = person(&service, root, "Go on.").await;
    ended(&service, next).await;
    assert!(text(&parent, 3).contains("four"));
    assert_eq!(
        service.background_status(root).unwrap().wake,
        AgentWakeState::On
    );
    finish(service, paths);
}

/// With `wakeOnBackgroundTasks` off the parent is told to wait, as before
/// wakes; its agent's end waits in its queue for `wait_agent`, no woken
/// turn starts, and its next turn does not read it.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_option_off_nothing_is_woken_and_wait_agent_takes_the_status() {
    let parent = FakeServer::start(vec![
        (
            spawn_on_second("counter", "Count the files."),
            Duration::ZERO,
        ),
        (assistant_events("The counter is on it."), Duration::ZERO),
        (assistant_events("Still here."), Duration::ZERO),
    ])
    .await;
    let children =
        FakeServer::start(vec![(assistant_events("Twelve files."), Duration::ZERO)]).await;
    let (service, paths, _, woken) = waking(
        "wake-off",
        &parent,
        &children,
        wake_on_background_tasks(false),
    )
    .await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let subagents = subagents(&service).await;

    let first = person(&service, root, "Delegate the count.").await;
    ended(&service, first).await;
    let guided = text(&parent, 0);
    assert!(guided.contains("If sub-agents are running, **wait for them before yielding** (`wait_agent`), unless the user asks an explicit question."));
    assert!(!guided.contains("You do not have to wait"));
    agents_done(&subagents, root).await;
    queued(&service, root, 1).await;
    assert_eq!(
        service.background_status(root).unwrap().wake,
        AgentWakeState::Off
    );
    assert_eq!(
        service
            .submit_wake_turn_with_sink(root, Arc::new(Discard))
            .await
            .unwrap(),
        None,
        "no woken turn"
    );
    let next = person(&service, root, "Anything?").await;
    ended(&service, next).await;
    assert!(!text(&parent, 2).contains(UPDATES));
    assert!(matches!(
        subagents.wait(root, NO_WAIT, &CancellationToken::default()).await.unwrap(),
        WaitOutcome::Updates(updates)
            if updates[0].status == AgentRunStatus::Completed(Some("Twelve files.".to_string()))
    ));
    assert_eq!(woken.woken(root), 0);
    finish(service, paths);
}

/// The option turned off while an agent works: its end wakes nothing, and
/// a host's submission starts no turn.
#[tokio::test(flavor = "multi_thread")]
async fn the_option_turned_off_while_an_agent_works_wakes_nothing() {
    let parent = FakeServer::start(vec![
        (
            spawn_on_second("counter", "Count the files."),
            Duration::ZERO,
        ),
        (assistant_events("The counter is on it."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(vec![(
        held(assistant_events("Twelve files.")),
        Duration::ZERO,
    )])
    .await;
    let (service, paths, config, woken) =
        waking("wake-turned-off", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let subagents = subagents(&service).await;
    let first = person(&service, root, "Delegate the count.").await;
    ended(&service, first).await;

    wake_on_background_tasks(false)(&config);
    children.hold.notify_one();
    agents_done(&subagents, root).await;
    queued(&service, root, 1).await;
    assert_eq!(
        service.background_status(root).unwrap().wake,
        AgentWakeState::Off
    );
    assert_eq!(
        service
            .submit_wake_turn_with_sink(root, Arc::new(Discard))
            .await
            .unwrap(),
        None
    );
    assert_eq!(woken.woken(root), 0);
    finish(service, paths);
}

/// An archived parent takes no event: what it held stays queued until it
/// is restored, and nothing new is queued meanwhile.
#[tokio::test(flavor = "multi_thread")]
async fn an_archived_parent_keeps_its_queue_and_takes_nothing_new() {
    let parent = FakeServer::start(Vec::new()).await;
    let children = FakeServer::start(Vec::new()).await;
    let (service, paths, _, _) = waking("wake-archived", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    set_wake_policy(&service, root.id, AgentWakePolicy::On)
        .await
        .unwrap();
    queue_wake_event(&service, root.id, completed("counter", "kept"))
        .await
        .unwrap();
    let root = service.snapshot(root.id).unwrap();
    let archived = service
        .archive_session(root.id, root.generation, root.revision)
        .unwrap();

    assert_eq!(
        queue_wake_event(&service, root.id, completed("docs", "dropped")).await,
        Err(hanafubuki_agent_kernel::AgentError::SessionArchived)
    );
    service
        .restore_session(root.id, archived.generation, archived.revision)
        .unwrap();
    assert_eq!(service.snapshot(root.id).unwrap().pending_wake_events, 1);
    finish(service, paths);
}

/// A side conversation is never woken: its policy is off.
#[tokio::test(flavor = "multi_thread")]
async fn a_side_conversation_is_never_woken() {
    let parent = FakeServer::start(vec![
        (assistant_events("Main."), Duration::ZERO),
        (assistant_events("Side."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(Vec::new()).await;
    let (service, paths, _, _) = waking("wake-side", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let first = person(&service, root, "Hello.").await;
    ended(&service, first).await;
    let side = service.open_side_conversation(root).unwrap().id;
    let asked = person(&service, side, "A question aside.").await;
    ended(&service, asked).await;

    assert_eq!(
        service.snapshot(side).unwrap().wake_policy,
        AgentWakePolicy::Off
    );
    assert_eq!(
        service.background_status(side).unwrap().wake,
        AgentWakeState::Off
    );
    assert_eq!(
        service.snapshot(root).unwrap().wake_policy,
        AgentWakePolicy::On
    );
    finish(service, paths);
}

/// A woken turn runs in the session's mode: in Plan mode it reads the
/// events with Plan mode's tools (it waits and lists, spawns nothing).
#[tokio::test(flavor = "multi_thread")]
async fn a_woken_turn_in_plan_mode_reads_the_events_with_plan_tools() {
    let parent = FakeServer::start(vec![(assistant_events("Planned."), Duration::ZERO)]).await;
    let children = FakeServer::start(Vec::new()).await;
    let (service, paths, _, _) = waking("wake-plan", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Plan, None)
        .await
        .unwrap()
        .id;
    set_wake_policy(&service, root, AgentWakePolicy::On)
        .await
        .unwrap();
    queue_wake_event(&service, root, completed("counter", "Twelve files."))
        .await
        .unwrap();

    let wake = service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .unwrap();
    ended(&service, wake.turn).await;
    let read = text(&parent, 0);
    assert!(read.contains(UPDATES) && read.contains("Twelve files."));
    assert!(read.contains(r#""name":"wait_agent""#));
    assert!(!read.contains(r#""name":"spawn_agent""#));
    finish(service, paths);
}

/// A goal lets the turn end while an agent works (its continuation passes),
/// and goes on in the woken turn once the agent reported.
#[tokio::test(flavor = "multi_thread")]
async fn a_goal_lets_the_turn_end_while_an_agent_works_and_goes_on_when_woken() {
    let parent = FakeServer::start(vec![
        (
            spawn_on_second("counter", "Count the files."),
            Duration::ZERO,
        ),
        (
            held(assistant_events("Waiting on the counter.")),
            Duration::ZERO,
        ),
        (assistant_events("The counter reported."), Duration::ZERO),
        (
            tool_events("update_goal", r#"{"status":"complete"}"#),
            Duration::ZERO,
        ),
        (assistant_events("Counted: twelve."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(vec![(
        held(assistant_events("Twelve files.")),
        Duration::ZERO,
    )])
    .await;
    let (service, paths, _, woken) = waking("wake-goal", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .set_goal(
            root,
            "Count the files".to_string(),
            AgentGoalBudgets::default(),
        )
        .await
        .unwrap();
    let goal_outcomes = |service: &AgentApplicationService| {
        service
            .events_since(root, None)
            .unwrap()
            .events
            .into_iter()
            .filter_map(|event| match event.kind {
                crate::AgentEventKind::ContinuationHookCompleted {
                    provider_id,
                    outcome,
                    ..
                } if provider_id == "goal" => Some(outcome),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    // The answer comes once the agent's turn runs, which the continuation
    // counts.
    let first = person(&service, root, "Count them with an agent.").await;
    wait_until("the agent samples", || children.requests().len() == 1).await;
    parent.hold.notify_one();
    ended(&service, first).await;
    assert_eq!(parent.requests().len(), 2, "the turn ended at its answer");
    assert_eq!(
        goal_outcomes(&service),
        [AgentContinuationHookOutcome::Passed]
    );

    children.hold.notify_one();
    wait_until("the parent is woken", || woken.woken(root) >= 1).await;
    let wake = service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .unwrap();
    ended(&service, wake.turn).await;
    assert_eq!(parent.requests().len(), 5);
    assert!(text(&parent, 3).contains("Continue working toward the active session goal."));
    assert_eq!(
        goal_outcomes(&service),
        [
            AgentContinuationHookOutcome::Passed,
            AgentContinuationHookOutcome::Continued
        ]
    );
    finish(service, paths);
}
