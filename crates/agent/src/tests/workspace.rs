//! Moving a running Agent to new workspace roots, as
//! `AgentRuntime::set_working_directory` does: refused, with nothing
//! changed, while a turn or a background terminal runs; then the filesystem
//! tools' policy and the commands' working directory follow the new roots.

use std::path::{Path, PathBuf};

use hanafubuki_agent_approval::ApprovalRiskClass;
use hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort;
use hanafubuki_agent_execution::AgentExecutionService;
use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_kernel::AgentSessionMode;
use hanafubuki_agent_workspace_fs::{ApplyPatchRequest, WorkspaceFsRequest};

use super::sessions::install_fake_background_execution;
use super::*;

fn native(path: &Path) -> PathBuf {
    ResolvedPath::resolve(path).unwrap().native().to_path_buf()
}

/// The risk of a patch adding `file`: high inside the workspace, critical
/// outside it.
fn patch_risk(service: &AgentApplicationService, file: &Path) -> ApprovalRiskClass {
    let patch = format!(
        "*** Begin Patch\n*** Add File: {}\n+new\n*** End Patch\n",
        file.display()
    );
    service
        .files
        .prepare(&WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest { patch }))
        .unwrap()
        .action()
        .risk
}

/// A directory outside the test host's workspace (and its protected
/// application data), as roots.
fn elsewhere() -> (tempfile::TempDir, PathBuf, crate::AgentWorkspaceRoots) {
    let temporary = tempfile::tempdir().unwrap();
    let directory = native(temporary.path());
    let roots = crate::agent_workspace_roots_from([directory.clone()]).unwrap();
    (temporary, directory, roots)
}

#[tokio::test]
async fn the_workspace_moves_once_no_background_terminal_runs_and_the_tools_follow() {
    let (mut service, paths) = configured_service("workspace-move", "http://127.0.0.1:1/v1");
    service.test_replace_executions(AgentExecutionService::with_process_port(
        FakeAgentExecutionProcessPort::shared(),
    ));
    let old_root = native(&paths.managed_instances_dir);
    let (_temporary, new_root, roots) = elsewhere();
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    install_fake_background_execution(&service, session.id, "sleep 100");

    assert_eq!(
        service.move_workspace(&roots).unwrap_err(),
        format!(
            "the working directory cannot change while 1 background terminal(s) run in session {}",
            session.id
        )
    );
    assert_eq!(
        patch_risk(&service, &new_root.join("new.txt")),
        ApprovalRiskClass::Critical
    );
    assert_eq!(
        service.test_executions().working_directory(),
        Some(old_root.clone())
    );

    service
        .stop_background_terminals_confirmed(session.id)
        .unwrap();
    service.move_workspace(&roots).unwrap();

    assert_eq!(
        patch_risk(&service, &new_root.join("new.txt")),
        ApprovalRiskClass::High
    );
    // A relative path resolves in the new root: in the old one, the test
    // host's workspace, it would be inside its protected data.
    assert_eq!(
        patch_risk(&service, Path::new("relative.txt")),
        ApprovalRiskClass::High
    );
    let (_outside, outside, _) = elsewhere();
    assert_eq!(
        patch_risk(&service, &outside.join("new.txt")),
        ApprovalRiskClass::Critical
    );
    assert_eq!(
        service.test_executions().working_directory(),
        Some(new_root)
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

// The turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn the_workspace_does_not_move_while_a_turn_runs() {
    let server = FakeServer::start(vec![(
        assistant_events("Done."),
        Duration::from_millis(1_500),
    )])
    .await;
    let (service, paths) = configured_service("workspace-turn", &server.base_url);
    let (_temporary, _, roots) = elsewhere();
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let running = service.clone();
    let turn = tokio::spawn(async move {
        running
            .start_turn(session.id, "Take your time".to_string().into())
            .await
    });
    let deadline = Instant::now() + TEST_WAIT;
    while service
        .snapshot(session.id)
        .unwrap()
        .turn
        .is_none_or(|turn| turn.phase.is_terminal())
    {
        assert!(Instant::now() < deadline, "the turn did not start");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    assert_eq!(
        service.move_workspace(&roots).unwrap_err(),
        format!(
            "the working directory cannot change while a turn runs in session {}",
            session.id
        )
    );
    turn.await.unwrap().unwrap();
    service.move_workspace(&roots).unwrap();
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
