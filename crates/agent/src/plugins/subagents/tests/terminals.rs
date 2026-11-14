//! A leaf's real background terminal holds the entire delegated tree open.

use super::wake::{queued, spawn_on_second, waking};
use super::*;
use crate::test_support::set_wake_policy;
use hanafubuki_agent_kernel::AgentWakePolicy;

#[tokio::test(flavor = "multi_thread")]
async fn a_leaf_terminal_holds_both_agents_until_their_woken_answers_reach_the_root() {
    let go = std::env::temp_dir().join(format!("nested-terminal-go-{}", std::process::id()));
    let _ = std::fs::remove_file(&go);
    #[cfg(not(windows))]
    let command = format!(
        "while [ ! -e '{}' ]; do sleep 0.1; done; echo leaf-terminal-finished",
        go.display()
    );
    #[cfg(windows)]
    let command = format!("while (-not (Test-Path -LiteralPath '{}')) {{ Start-Sleep -Milliseconds 100 }}; Write-Output leaf-terminal-finished", go.display());
    let arguments = json!({"cmd":command,"yield_time_ms":500}).to_string();
    let branch_model = FakeServer::start(vec![
        (
            spawn_on_second("leaf", "Start the command."),
            Duration::ZERO,
        ),
        (assistant_events("The leaf is working."), Duration::ZERO),
        (assistant_events("All nested work is done."), Duration::ZERO),
    ])
    .await;
    let leaf_model = FakeServer::start(vec![
        (
            tool_calls_events(&[("call-terminal", "exec_command", &arguments)]),
            Duration::ZERO,
        ),
        (assistant_events("The terminal is working."), Duration::ZERO),
        (
            assistant_events("The leaf command finished."),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths, _, events) =
        waking("nested-terminal", &branch_model, &leaf_model, |_| {}).await;
    let root = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .set_session_permission_mode(root, AgentPermissionMode::FullAccess)
        .unwrap();
    service
        .set_session_sandbox_mode(root, crate::AgentSandboxMode::FullAccess)
        .unwrap();
    set_wake_policy(&service, root, AgentWakePolicy::On)
        .await
        .unwrap();
    let agents = subagents(&service).await;
    agents
        .spawn(root, spawn("branch", "Delegate the command."))
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let tasks = loop {
        let tasks = service.background_tasks(root).await.unwrap();
        if tasks.agents.len() == 2
            && tasks
                .agents
                .iter()
                .all(|agent| agent.status == AgentSubagentState::Waiting)
        {
            break tasks;
        }
        assert!(
            Instant::now() < deadline,
            "both agents did not wait: {tasks:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!((tasks.work.agents, tasks.work.terminals), (2, 1));
    assert_eq!(tasks.terminals.len(), 1);
    assert_eq!(tasks.terminals[0].owner, tasks.agents[1].session_id);
    assert_eq!(service.core.background_work(root), tasks.work);
    assert_eq!(service.background_status(root).unwrap().work, tasks.work);
    assert_eq!(tasks.pending_wake_events, 0);
    assert_eq!(events.woken(root), 0);
    std::fs::write(&go, "go").unwrap();
    queued(&service, root, 1).await;
    let updates = agents
        .wait(root, Duration::ZERO, &CancellationToken::default())
        .await
        .unwrap();
    assert!(
        matches!(updates, WaitOutcome::Updates(ref updates) if updates.len() == 1 && updates[0].status == AgentRunStatus::Completed(Some("All nested work is done.".into())))
    );
    assert_eq!(leaf_model.requests().len(), 3);
    assert_eq!(branch_model.requests().len(), 3);
    assert!(body(&leaf_model, 2).contains("leaf-terminal-finished"));
    assert!(body(&branch_model, 2).contains("The leaf command finished."));
    assert!(service.background_status(root).unwrap().work.is_empty());
    assert_eq!(
        agents
            .wait(root, Duration::ZERO, &CancellationToken::default())
            .await
            .unwrap(),
        WaitOutcome::Idle
    );
    finish(service, paths);
    let _ = std::fs::remove_file(go);
}
