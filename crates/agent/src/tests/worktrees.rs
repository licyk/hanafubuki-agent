//! The worktree calls: worktrees of the repository the Agent works in,
//! from the `worktree` plugin row, under the data directory; with the row
//! off they are unavailable.

use std::path::Path;
use std::process::Command;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_kernel::{AgentError, AgentSessionId};

use super::host::AgentPluginState;
use super::*;

fn setup_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

/// The test host's workspace as a repository with one commit.
fn repository(paths: &TestPaths) {
    let workspace = &paths.managed_instances_dir;
    std::fs::write(workspace.join("file.txt"), "committed\n").unwrap();
    setup_git(workspace, &["init", "-q"]);
    setup_git(workspace, &["add", "."]);
    setup_git(
        workspace,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "-m",
            "initial",
        ],
    );
}

// The worktree service's blocking work must finish while `shutdown` blocks
// this thread.
#[tokio::test(flavor = "multi_thread")]
async fn worktrees_are_made_listed_bound_and_removed_through_their_plugin() {
    let (service, paths) = configured_service("worktree-calls", "http://127.0.0.1:1/v1");
    repository(&paths);
    let (owner, other) = (AgentSessionId::new(7), AgentSessionId::new(8));

    let first = service.worktree_create(Some(owner)).await.unwrap();
    let second = service.worktree_create(None).await.unwrap();

    assert_eq!(first.owner, Some(owner));
    assert_eq!(second.owner, None);
    assert_eq!(
        first.path.parent().unwrap(),
        ResolvedPath::resolve(paths.data_dir.join("agent-test/worktrees"))
            .unwrap()
            .native()
    );
    assert_eq!(
        std::fs::read_to_string(first.path.join("file.txt")).unwrap(),
        "committed\n"
    );
    service
        .worktree_bind(second.path.clone(), other)
        .await
        .unwrap();
    let error = service
        .worktree_bind(first.path.clone(), other)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        AgentError::InvalidInput("the worktree belongs to session 7".to_string())
    );
    let mut listed = service.worktree_list().await.unwrap();
    listed.sort_by(|left, right| left.path.cmp(&right.path));
    let mut expected = vec![
        first.clone(),
        AgentWorktree {
            owner: Some(other),
            ..second.clone()
        },
    ];
    expected.sort_by(|left, right| left.path.cmp(&right.path));
    assert_eq!(listed, expected);

    service.worktree_remove(first.path.clone()).await.unwrap();
    assert!(!first.path.exists());
    assert_eq!(service.worktree_list().await.unwrap().len(), 1);

    service
        .set_plugin_enabled("worktree".into(), false)
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service
        .plugin_status()
        .into_iter()
        .any(|status| status.name == "worktree" && status.state != AgentPluginState::Disabled)
    {
        assert!(Instant::now() < deadline, "the worktree plugin stayed on");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        service.worktree_list().await.unwrap_err(),
        AgentError::FeatureUnavailable {
            feature: "worktree".to_string(),
            reason: "the worktree Agent plugin is unavailable: it is turned off".to_string(),
        }
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
