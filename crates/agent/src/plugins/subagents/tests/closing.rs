//! Closing a delegated subtree confirms every process stop before reporting it.

use super::wake::{held, queued, spawn_on_second, waking};
use super::*;
use crate::test_support::set_wake_policy;
use hanafubuki_agent_kernel::AgentWakePolicy;

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_waiting_parent_stops_its_child_and_both_terminals_then_accepts_a_followup() {
    let go = std::env::temp_dir().join(format!("closed-tree-go-{}", std::process::id()));
    let _ = std::fs::remove_file(&go);
    #[cfg(not(windows))]
    let command = format!("while [ ! -e '{}' ]; do sleep 0.1; done", go.display());
    #[cfg(windows)]
    let command = format!(
        "while (-not (Test-Path -LiteralPath '{}')) {{ Start-Sleep -Milliseconds 100 }}",
        go.display()
    );
    let arguments = json!({"cmd":command,"yield_time_ms":500}).to_string();
    let execution = tool_calls_events(&[("call-terminal", "exec_command", &arguments)]);
    let branch_model = FakeServer::start(vec![
        (
            spawn_on_second("leaf", "Start the command."),
            Duration::ZERO,
        ),
        (execution.clone(), Duration::ZERO),
        (assistant_events("I am waiting."), Duration::ZERO),
        (assistant_events("The new task is done."), Duration::ZERO),
    ])
    .await;
    let leaf_model = FakeServer::start(vec![
        (execution, Duration::ZERO),
        (
            held(assistant_events("The terminal is working.")),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths, _, events) =
        waking("close-tree", &branch_model, &leaf_model, |_| {}).await;
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
    assert!(matches!(
        service.close_subagent(root).await,
        Err(AgentError::InvalidInput(_))
    ));
    let agents = subagents(&service).await;
    let branch = agents
        .spawn(
            root,
            spawn("branch", "Delegate and start your own command."),
        )
        .await
        .unwrap()
        .id;
    let deadline = Instant::now() + Duration::from_secs(30);
    let leaf = loop {
        let tasks = service.background_tasks(root).await.unwrap();
        if tasks.agents.len() == 2
            && tasks.terminals.len() == 2
            && tasks.agents[0].status == AgentSubagentState::Waiting
            && tasks.agents[1].status == AgentSubagentState::Running
        {
            break tasks.agents[1].session_id;
        }
        assert!(Instant::now() < deadline, "tree did not wait: {tasks:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    service.close_subagent(branch).await.unwrap();
    queued(&service, root, 1).await;
    let tasks = service.background_tasks(root).await.unwrap();
    assert!(tasks
        .agents
        .iter()
        .all(|agent| agent.status == AgentSubagentState::Closed));
    assert!(tasks.terminals.is_empty());
    assert!(tasks.work.is_empty());
    for child in [branch, leaf] {
        assert_eq!(service.snapshot(child).unwrap().pending_wake_events, 0);
    }
    let update = agents
        .wait(root, Duration::ZERO, &CancellationToken::default())
        .await
        .unwrap();
    assert_eq!(
        update,
        WaitOutcome::Updates(vec![AgentUpdate {
            name: "branch".into(),
            id: branch,
            status: AgentRunStatus::Closed
        }])
    );
    assert_eq!(
        leaf_model.requests().len(),
        2,
        "the closing leaf was never resumed"
    );
    assert_eq!(
        branch_model.requests().len(),
        3,
        "the closing parent was never resumed"
    );
    assert!(events.woken(root) <= 1);
    agents
        .followup_task(root, "branch", "Do a new task.".into())
        .await
        .unwrap();
    queued(&service, root, 1).await;
    let update = agents
        .wait(root, Duration::ZERO, &CancellationToken::default())
        .await
        .unwrap();
    assert!(
        matches!(update, WaitOutcome::Updates(ref updates) if updates[0].status == AgentRunStatus::Completed(Some("The new task is done.".into())))
    );
    assert_eq!(branch_model.requests().len(), 4);
    finish(service, paths);
}
