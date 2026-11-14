//! Nested delegation against real scripted model requests and the dispatcher.

use super::wake::{held, queued, spawn_on_second, waking};
use super::*;
use crate::test_support::set_wake_policy;
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentSessionPersistence, AgentWakePolicy,
};

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_parent_is_reported_only_after_its_child_wakes_it_and_it_answers() {
    let first = FakeServer::start(vec![
        (
            spawn_on_second("leaf", "Do the nested work."),
            Duration::ZERO,
        ),
        (assistant_events("Waiting for the leaf."), Duration::ZERO),
        (
            assistant_events("The nested work is complete."),
            Duration::ZERO,
        ),
    ])
    .await;
    let second =
        FakeServer::start(vec![(held(assistant_events("Leaf done.")), Duration::ZERO)]).await;
    let (service, paths, _, events) = waking("nested-completion", &first, &second, |_| {}).await;
    let agents = subagents(&service).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    set_wake_policy(&service, root, AgentWakePolicy::On)
        .await
        .unwrap();
    let child = agents
        .spawn(root, spawn("branch", "Delegate the nested work."))
        .await
        .unwrap()
        .id;
    wait_for_state(&agents, root, child, AgentSubagentState::Waiting).await;
    let tasks = service.background_tasks(root).await.unwrap();
    assert_eq!(tasks.agents.len(), 2);
    assert_eq!(tasks.agents[0].status, AgentSubagentState::Waiting);
    assert_eq!(
        (tasks.agents[0].parent_session_id, tasks.agents[0].depth),
        (root, 1)
    );
    let leaf = tasks.agents[1].session_id;
    assert_eq!(
        (tasks.agents[1].parent_session_id, tasks.agents[1].depth),
        (child, 2)
    );
    assert_eq!(service.background_status(root).unwrap().work.agents, 2);
    assert_eq!(service.core.background_work(root).agents, 2);
    assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 0);
    assert_eq!(events.woken(root), 0);
    let listed = call(&agents, SubagentToolKind::ListAgents, child, json!({}))
        .await
        .unwrap();
    assert_eq!(listed["agents"].as_array().unwrap().len(), 1);
    assert_eq!(listed["agents"][0]["agent_name"], "leaf");
    let refused = agents
        .spawn(leaf, spawn("too_deep", "Cannot run."))
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("nest at most two levels"),
        "{refused}"
    );
    assert!(agents.agents(leaf).await.unwrap().is_empty());
    second.hold.notify_one();
    queued(&service, root, 1).await;
    let updates = agents
        .wait(root, NO_WAIT, &CancellationToken::default())
        .await
        .unwrap();
    assert_eq!(
        updates,
        WaitOutcome::Updates(vec![AgentUpdate {
            name: "branch".into(),
            id: child,
            status: AgentRunStatus::Completed(Some("The nested work is complete.".into()))
        }])
    );
    assert_eq!(first.requests().len(), 3);
    assert!(body(&first, 2).contains("<background_events>"));
    assert!(body(&first, 2).contains("Leaf done."));
    assert!(!body(&second, 0).contains("<subagents>"));
    assert_eq!(
        agents
            .wait(root, Duration::ZERO, &CancellationToken::default())
            .await
            .unwrap(),
        WaitOutcome::Idle
    );
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn eight_live_agents_include_a_waiting_parent_and_its_wake_ignores_the_cap() {
    cap_with_parent_outcome(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_parent_with_a_live_child_keeps_its_slot_and_can_be_woken_at_the_cap() {
    cap_with_parent_outcome(true).await;
}

async fn cap_with_parent_outcome(failed: bool) {
    let first = FakeServer::start(vec![
        (
            spawn_on_second("leaf", "Wait for a signal."),
            Duration::ZERO,
        ),
        (if failed { "event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_error\",\"message\":\"Failed after spawning\"}}}\n\n".into() } else { assistant_events("Waiting.") }, Duration::ZERO),
        (
            assistant_events("I read the progress message."),
            Duration::ZERO,
        ),
    ])
    .await;
    let second =
        FakeServer::start(vec![(held(assistant_events("Done.")), Duration::ZERO); 7]).await;
    let (service, paths, _, _) = waking("nested-cap", &first, &second, |_| {}).await;
    let agents = subagents(&service).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    set_wake_policy(&service, root, AgentWakePolicy::On)
        .await
        .unwrap();
    let child = agents
        .spawn(root, spawn("branch", "Delegate."))
        .await
        .unwrap()
        .id;
    if failed {
        queued(&service, root, 1).await;
        // The error is queued before the run stops counting as live.
        wait_for_state(&agents, root, child, AgentSubagentState::Failed).await;
        let updates = agents
            .wait(root, NO_WAIT, &CancellationToken::default())
            .await
            .unwrap();
        assert!(
            matches!(updates, WaitOutcome::Updates(ref updates) if matches!(updates[0].status, AgentRunStatus::Errored(_)))
        );
    } else {
        wait_for_state(&agents, root, child, AgentSubagentState::Waiting).await;
    }
    let leaf = agents.agents(child).await.unwrap()[0].id;
    for index in 0..6 {
        let mut request = spawn(&format!("peer_{index}"), "Wait.");
        request.provider = Some("provider-2".into());
        request.model = Some("fixture-model".into());
        agents.spawn(root, request).await.unwrap();
    }
    assert_eq!(service.background_status(root).unwrap().work.agents, 8);
    assert_eq!(service.background_tasks(root).await.unwrap().work.agents, 8);
    for parent in [root, child] {
        let refused = agents
            .spawn(parent, spawn("ninth", "Cannot run."))
            .await
            .unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("at most 8 sub-agents work at once under the main conversation"),
            "{refused}"
        );
    }
    agents
        .send_message(leaf, "parent", "Progress from the leaf.".into())
        .await
        .unwrap();
    wait_until("the wake requested", || first.requests().len() == 3).await;
    wait_for_state(&agents, root, child, AgentSubagentState::Waiting).await;
    assert!(body(&first, 2).contains("Progress from the leaf."));
    assert_eq!(service.background_status(root).unwrap().work.agents, 8);
    assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 0);
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn side_and_review_sessions_cannot_spawn() {
    let server = FakeServer::start(Vec::new()).await;
    let (service, paths) = started("nested-origins", &server).await;
    let agents = subagents(&service).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    for origin in [AgentSessionOrigin::Side, AgentSessionOrigin::Review] {
        let session = agents
            .driver
            .create_child(AgentChildSessionRequest {
                parent_session_id: root,
                origin,
                persistence: AgentSessionPersistence::Ephemeral,
                base: AgentChildSessionBase::Empty,
                context: Vec::new(),
                scope: Default::default(),
                role_instructions: None,
            })
            .unwrap();
        let refused = agents
            .spawn(session.id, spawn("child", "Cannot run."))
            .await
            .unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("this conversation cannot spawn agents"),
            "{refused}"
        );
        assert!(agents.agents(session.id).await.unwrap().is_empty());
    }
    assert!(server.requests().is_empty());
    finish(service, paths);
}

async fn wait_for_state(
    agents: &SubagentService,
    parent: AgentSessionId,
    child: AgentSessionId,
    expected: AgentSubagentState,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if agents
            .statuses(parent)
            .await
            .unwrap()
            .iter()
            .any(|agent| agent.session_id == child && agent.status == expected)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the child never reached {expected:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn resuming_the_root_rechecks_a_descendant_even_with_no_root_updates() {
    let first = FakeServer::start(vec![
        (spawn_on_second("leaf", "Wait."), Duration::ZERO),
        (assistant_events("Waiting."), Duration::ZERO),
        (assistant_events("I received the progress."), Duration::ZERO),
    ])
    .await;
    let second = FakeServer::start(vec![(held(assistant_events("Done.")), Duration::ZERO)]).await;
    let (service, paths, _, events) = waking("nested-resume", &first, &second, |_| {}).await;
    let agents = subagents(&service).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    set_wake_policy(&service, root, AgentWakePolicy::On)
        .await
        .unwrap();
    let child = agents
        .spawn(root, spawn("branch", "Delegate."))
        .await
        .unwrap()
        .id;
    wait_for_state(&agents, root, child, AgentSubagentState::Waiting).await;
    let leaf = agents.agents(child).await.unwrap()[0].id;
    agents.wakes.pauses.interrupted(root);
    agents
        .send_message(leaf, "parent", "Progress while paused.".into())
        .await
        .unwrap();
    wake::settled(&service, &events).await;
    assert_eq!(first.requests().len(), 2);
    assert_eq!(service.snapshot(root).unwrap().pending_wake_events, 0);
    assert_eq!(service.snapshot(child).unwrap().pending_wake_events, 1);
    service.resume_background_wakes(root).unwrap();
    wait_until("the descendant was resumed", || first.requests().len() == 3).await;
    wait_for_state(&agents, root, child, AgentSubagentState::Waiting).await;
    assert!(body(&first, 2).contains("Progress while paused."));
    assert_eq!(service.snapshot(child).unwrap().pending_wake_events, 0);
    assert_eq!(
        service.background_status(root).unwrap().wake,
        crate::AgentWakeState::On
    );
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_leaf_wakes_its_parent_and_only_the_parents_answer_reaches_the_root() {
    let first = FakeServer::start(vec![
        (spawn_on_second("leaf", "Try the work."), Duration::ZERO),
        (assistant_events("Waiting."), Duration::ZERO),
        (
            assistant_events("The leaf failed; I handled it."),
            Duration::ZERO,
        ),
    ])
    .await;
    let second = FakeServer::start(vec![(held(b"event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"invalid_request\",\"message\":\"leaf refused\"}}}\n\n".to_vec()), Duration::ZERO)]).await;
    let (service, paths, _, events) = waking("nested-failure", &first, &second, |_| {}).await;
    let agents = subagents(&service).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    set_wake_policy(&service, root, AgentWakePolicy::On)
        .await
        .unwrap();
    let child = agents
        .spawn(root, spawn("branch", "Delegate."))
        .await
        .unwrap()
        .id;
    wait_for_state(&agents, root, child, AgentSubagentState::Waiting).await;
    second.hold.notify_one();
    queued(&service, root, 1).await;
    assert!(body(&first, 2).contains("leaf refused"));
    assert_eq!(
        events.woken(child),
        0,
        "the dispatcher runs children, not the host"
    );
    let updates = agents
        .wait(root, NO_WAIT, &CancellationToken::default())
        .await
        .unwrap();
    assert_eq!(
        updates,
        WaitOutcome::Updates(vec![AgentUpdate {
            name: "branch".into(),
            id: child,
            status: AgentRunStatus::Completed(Some("The leaf failed; I handled it.".into()))
        }])
    );
    finish(service, paths);
}
