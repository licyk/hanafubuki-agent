//! The worktree plugin provides its service under its name while it runs,
//! serves worktrees through it, and withdraws it when it stops.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_foundation::blocking::InlineBlockingOperationPort;
use hanafubuki_agent_plugin_worktree::{worktree_plugin, WorktreeService, WORKTREE_PLUGIN};
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_worktree::WorktreeManager;
use kardia_core::{Context, FiberState, PluginDef, Service};
use serde_json::Value;

fn setup_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[tokio::test]
async fn provides_the_worktree_service_while_it_runs() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("project");
    std::fs::create_dir_all(&repository).unwrap();
    setup_git(&repository, &["init", "-q"]);
    setup_git(
        &repository,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    let root = Context::new();
    let host = PluginDef::new(|ctx: Context, _| async move {
        ctx.provide(BlockingService::new(Arc::new(InlineBlockingOperationPort)))?;
        Ok(())
    })
    .provides([BlockingService::NAME]);
    root.plugin_def(&host, Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    let worktrees = WorktreeManager::new(
        directory.path().join("worktrees"),
        ExecutorManager::default(),
    );
    let fiber = root
        .plugin_def(&worktree_plugin(worktrees), Value::Null)
        .unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);

    let service = root
        .live_as::<WorktreeService>(WORKTREE_PLUGIN)
        .unwrap()
        .arc()
        .clone();
    let created = service.create(repository.clone()).await.unwrap();
    assert_eq!(service.list(repository.clone()).await.unwrap(), [created]);
    let error = service
        .create(directory.path().join("worktrees"))
        .await
        .unwrap_err();
    assert!(error.contains("not a git repository"), "{error}");

    fiber.dispose().await.unwrap();
    assert!(root.live_as::<WorktreeService>(WORKTREE_PLUGIN).is_none());
    root.fiber().dispose().await.unwrap();
}
