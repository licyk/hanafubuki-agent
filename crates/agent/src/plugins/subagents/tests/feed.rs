//! What the host is told of a root's background work, over two scripted
//! models (the parent on one, its agents on the other, so a test can hold
//! either): `Changed` once for each change of what `background_tasks`
//! answers (counted where the scenario holds still), never while nothing
//! changes, also for an agent interrupted (it reports nothing), a pause and
//! the option; a sub-agent's turn's start, approval, question and end as
//! `Child` events, and nothing else of it; and `background_tasks` agreeing
//! with `background_status` at every step, an agent that ended always
//! running or reported in it.

use hanafubuki_agent_kernel::{
    AgentEventKind, AgentSessionTitleRenameRequest, AgentSubagentWake, AgentUserInputAnswer,
    AgentWakePolicy, AgentWakeSource, ApprovalUserDecision,
};

use super::wake::{agents_done, ended, held, person, queued, spawn_on_second, waking, Discard};
use super::*;
use crate::test_support::{
    queue_wake_event, set_wake_policy, tool_events, RecordedBackgroundEvents,
};
use crate::{
    AgentBackgroundTasks, AgentSessionModelSelectionRequest, AgentSettingsPatch, AgentWakeState,
};

/// Waits until the feed has looked at every hint sent so far: it looks at
/// the roots hinted together in the order of their ids, so a root created
/// and changed now is told after the earlier ones were looked at.
async fn feed_settled(service: &AgentApplicationService, told: &RecordedBackgroundEvents) {
    let probe = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    set_wake_policy(service, probe, AgentWakePolicy::On)
        .await
        .unwrap();
    let message = AgentWakeSource::Subagent {
        session_id: probe,
        name: "probe".to_string(),
        event: AgentSubagentWake::Message {
            text: "probe".to_string(),
        },
    };
    queue_wake_event(service, probe, message).await.unwrap();
    wait_until("the feed reaches the probe", || told.changed(probe) == 1).await;
}

/// `root`'s tree, checked against its counts: its running agents are the
/// ones counted, and the queue and the wake are the same.
async fn tree(service: &AgentApplicationService, root: AgentSessionId) -> AgentBackgroundTasks {
    let tasks = service.background_tasks(root).await.unwrap();
    let status = service.background_status(root).unwrap();
    let running = running(&tasks);
    assert_eq!(
        (running, tasks.pending_wake_events, tasks.wake),
        (status.work.agents, status.pending_wake_events, status.wake)
    );
    tasks
}

fn running(tasks: &AgentBackgroundTasks) -> usize {
    tasks
        .agents
        .iter()
        .filter(|agent| agent.status == AgentSubagentState::Running)
        .count()
}

/// A root that delegates, is told its agent's end, reads it in a woken
/// turn, and then sees its agent's model and title changed by a person:
/// one `Changed` for each, two at most for the agent's end (its report is
/// queued, then it stops running), none while nothing changes.
#[tokio::test(flavor = "multi_thread")]
async fn the_host_is_told_once_for_each_change_of_the_tree() {
    let parent = FakeServer::start(vec![
        (
            held(spawn_on_second("counter", "Count the files.")),
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
    let (service, paths, _, told) = waking("feed-changes", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    // The person's turn changes nothing in the background.
    let first = person(&service, root, "Delegate the count.").await;
    wait_until("the parent asks", || parent.requests().len() == 1).await;
    feed_settled(&service, &told).await;
    assert_eq!(told.changed(root), 0);

    parent.hold.notify_one();
    ended(&service, first).await;
    feed_settled(&service, &told).await;
    assert_eq!(told.changed(root), 1, "the agent started");
    let started = tree(&service, root).await;
    assert_eq!(running(&started), 1);
    assert_eq!(started.agents[0].title, "counter");
    assert!(started.agents[0].state_since.is_some());
    assert!(started.terminals.is_empty());

    // An agent that ended is running or reported in every read.
    let reader = service.clone();
    let reads = tokio::spawn(async move {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            assert!(Instant::now() < deadline, "the agent's end was not read");
            let tasks = reader.background_tasks(root).await.unwrap();
            assert!(
                running(&tasks) + tasks.pending_wake_events >= 1,
                "neither running nor reported: {tasks:?}"
            );
            if tasks.pending_wake_events == 1 && running(&tasks) == 0 {
                return;
            }
        }
    });
    children.hold.notify_one();
    reads.await.unwrap();
    wait_until("the parent is woken", || told.woken(root) >= 1).await;
    feed_settled(&service, &told).await;
    let ending = told.changed(root);
    assert!((2..=3).contains(&ending), "{ending} events for the end");
    let reported = tree(&service, root).await;
    assert_eq!((running(&reported), reported.pending_wake_events), (0, 1));
    assert_eq!(reported.agents[0].status, AgentSubagentState::Idle);
    assert_eq!(
        reported.agents[0].last_answer.as_deref(),
        Some("Twelve files.")
    );

    // The woken turn reads the report.
    let woken = service
        .submit_wake_turn_with_sink(root, Arc::new(Discard))
        .await
        .unwrap()
        .unwrap();
    ended(&service, woken.turn).await;
    feed_settled(&service, &told).await;
    assert_eq!(told.changed(root), ending + 1, "the report was read");
    assert_eq!(tree(&service, root).await.pending_wake_events, 0);

    // A person's /model and rename of the agent.
    let child = reported.agents[0].session_id;
    let session = service.snapshot(child).unwrap();
    service
        .set_session_model(AgentSessionModelSelectionRequest {
            session_id: child,
            generation: session.generation,
            revision: session.revision,
            provider_id: "provider-2".to_string(),
            model: "moved-model".to_string(),
        })
        .await
        .unwrap();
    feed_settled(&service, &told).await;
    assert_eq!(told.changed(root), ending + 2, "the model changed");
    assert_eq!(tree(&service, root).await.agents[0].model, "moved-model");
    let session = service.snapshot(child).unwrap();
    service
        .rename_session(
            AgentSessionTitleRenameRequest {
                session_id: child,
                generation: session.generation,
                revision: session.revision,
                title: "tally".to_string(),
            },
            &Discard,
        )
        .unwrap();
    feed_settled(&service, &told).await;
    assert_eq!(told.changed(root), ending + 3, "the title changed");
    assert_eq!(tree(&service, root).await.agents[0].title, "tally");

    feed_settled(&service, &told).await;
    assert_eq!(told.changed(root), ending + 3, "nothing changed");
    finish(service, paths);
}

/// An agent interrupted reports nothing to its parent, and stops running:
/// that is a change too, which a host waiting for the tree must hear of.
#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_agent_is_a_change_though_it_reports_nothing() {
    let parent = FakeServer::start(vec![
        (
            spawn_on_second("counter", "Count the files."),
            Duration::ZERO,
        ),
        (assistant_events("The counter is on it."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(vec![(
        held(assistant_events("Never read.")),
        Duration::ZERO,
    )])
    .await;
    let (service, paths, _, told) = waking("feed-interrupt", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let first = person(&service, root, "Delegate the count.").await;
    ended(&service, first).await;
    wait_until("the agent's turn runs", || children.requests().len() == 1).await;
    feed_settled(&service, &told).await;
    let started = told.changed(root);
    let child = tree(&service, root).await.agents[0].session_id;

    service.interrupt_subagent(child).await.unwrap();
    agents_done(&subagents(&service).await, root).await;
    feed_settled(&service, &told).await;
    assert_eq!(told.changed(root), started + 1);
    let stopped = tree(&service, root).await;
    assert_eq!((running(&stopped), stopped.pending_wake_events), (0, 0));
    assert_eq!(service.background_status(root).unwrap().work.agents, 0);
    children.hold.notify_one();
    finish(service, paths);
}

/// The person's interrupt pauses the root's wakes and their next message
/// resumes them; turning `wakeOnBackgroundTasks` off turns them off: each
/// is a change of what `background_tasks` answers.
#[tokio::test(flavor = "multi_thread")]
async fn a_pause_its_end_and_the_option_are_changes() {
    let parent = FakeServer::start(vec![
        (held(assistant_events("Never read.")), Duration::ZERO),
        (assistant_events("Again."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(Vec::new()).await;
    let (service, paths, config, told) = waking("feed-pause", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    let turn = person(&service, root, "Wait.").await;
    wait_until("the parent asks", || parent.requests().len() == 1).await;
    service
        .cancel_turn(root, turn.turn_id, turn.generation)
        .await
        .unwrap();
    ended(&service, turn).await;
    parent.hold.notify_one();
    feed_settled(&service, &told).await;
    assert_eq!(told.changed(root), 1, "paused");
    assert!(matches!(
        tree(&service, root).await.wake,
        AgentWakeState::Paused { .. }
    ));

    let next = person(&service, root, "Again.").await;
    ended(&service, next).await;
    feed_settled(&service, &told).await;
    assert_eq!(told.changed(root), 2, "resumed");
    assert_eq!(tree(&service, root).await.wake, AgentWakeState::On);

    config
        .write_settings(AgentSettingsPatch {
            wake_on_background_tasks: Some(false),
            ..AgentSettingsPatch::default()
        })
        .unwrap();
    service.apply_settings_to_plugins();
    feed_settled(&service, &told).await;
    assert_eq!(told.changed(root), 3, "turned off");
    assert_eq!(tree(&service, root).await.wake, AgentWakeState::Off);
    finish(service, paths);
}

/// In Manual mode an agent's command and its question wait on a person:
/// the host is told each as a `Child` event of the root, answers it on the
/// agent, and is told the answers and the turn's start and end, nothing
/// else of the agent's turn.
#[tokio::test(flavor = "multi_thread")]
async fn an_agents_approval_and_question_reach_the_host_as_child_events() {
    let command = json!({
        "cmd": "echo child-output",
        "shell": null,
        "login": null,
        "tty": null,
        "yield_time_ms": 30_000,
        "max_output_bytes": 32_768,
        "workdir": null,
    });
    let question = json!({
        "questions": [{
            "id": "scope",
            "header": "Scope",
            "question": "Choose a scope",
            "options": [
                { "id": "focused", "label": "Focused", "description": "Only this" },
                { "id": "broad", "label": "Broad", "description": "All of it" }
            ]
        }]
    });
    let parent = FakeServer::start(vec![
        (spawn_on_second("runner", "Run it."), Duration::ZERO),
        (assistant_events("The runner is on it."), Duration::ZERO),
    ])
    .await;
    let children = FakeServer::start(vec![
        (
            tool_events("exec_command", &command.to_string()),
            Duration::ZERO,
        ),
        (
            tool_events("request_user_input", &question.to_string()),
            Duration::ZERO,
        ),
        (assistant_events("Ran it, focused."), Duration::ZERO),
    ])
    .await;
    let (service, paths, _, told) = waking("feed-child", &parent, &children, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .set_session_permission_mode(root, AgentPermissionMode::Manual)
        .unwrap();
    let first = person(&service, root, "Delegate the run.").await;
    ended(&service, first).await;

    let asked = |kind: &str| {
        let kind = kind.to_string();
        let told = told.clone();
        move || {
            told.child_events(root)
                .iter()
                .any(|event| kind_name(&event.kind) == kind)
        }
    };
    wait_until("the approval is told", asked("approval_requested")).await;
    let approval = told
        .child_events(root)
        .into_iter()
        .find(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. }))
        .unwrap();
    let AgentEventKind::ApprovalRequested { request } = &approval.kind else {
        unreachable!();
    };
    service
        .resolve_approval(
            approval.session_id,
            approval.turn_id.unwrap(),
            approval.generation,
            request.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    wait_until("the question is told", asked("user_input_requested")).await;
    let asking = told
        .child_events(root)
        .into_iter()
        .find(|event| matches!(event.kind, AgentEventKind::UserInputRequested { .. }))
        .unwrap();
    let AgentEventKind::UserInputRequested { request } = &asking.kind else {
        unreachable!();
    };
    service
        .resolve_user_input(
            asking.session_id,
            asking.turn_id.unwrap(),
            asking.generation,
            request.request_id,
            vec![AgentUserInputAnswer {
                question_id: "scope".to_string(),
                option_id: "focused".to_string(),
            }],
        )
        .unwrap();
    wait_until("the agent's turn ends", asked("turn_completed")).await;
    queued(&service, root, 1).await;

    let events = told.child_events(root);
    assert_eq!(
        events
            .iter()
            .map(|event| kind_name(&event.kind))
            .collect::<Vec<_>>(),
        [
            "turn_started",
            "approval_requested",
            "approval_resolved",
            "user_input_requested",
            "user_input_resolved",
            "turn_completed"
        ]
    );
    let child = tree(&service, root).await.agents[0].session_id;
    assert!(events.iter().all(|event| event.session_id == child));
    let answered = String::from_utf8(children.requests()[1].body.clone()).unwrap();
    assert!(answered.contains("child-output"), "{answered}");
    finish(service, paths);
}

/// The kind's name, as the events are serialized.
fn kind_name(kind: &AgentEventKind) -> String {
    serde_json::to_value(kind).unwrap()["type"]
        .as_str()
        .unwrap()
        .to_string()
}
