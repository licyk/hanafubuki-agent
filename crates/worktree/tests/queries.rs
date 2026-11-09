//! The review queries against a real repository: branches with the default
//! first, the current branch, recent commits and the merge base, each `git`
//! through the executor and disposed of.

use std::path::Path;
use std::process::Command;

use hanafubuki_agent_process::executor::manager::executor_registry_counts;
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_worktree::{GitCommit, GitQueries, WorktreeError};

fn setup_git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn commit(cwd: &Path, message: &str) -> String {
    setup_git(
        cwd,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            message,
        ],
    );
    setup_git(cwd, &["rev-parse", "HEAD"])
}

/// `main` with commits `base` and `second`, and `feature` from `base` with
/// `feature work`, checked out.
struct Repository {
    _directory: tempfile::TempDir,
    executor: ExecutorManager,
    queries: GitQueries,
    base: String,
    second: String,
    feature: String,
}

fn repository() -> Repository {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    setup_git(cwd, &["init", "-q", "-b", "main"]);
    let base = commit(cwd, "base");
    setup_git(cwd, &["branch", "feature"]);
    setup_git(cwd, &["branch", "alpha"]);
    let second = commit(cwd, "second");
    setup_git(cwd, &["checkout", "-q", "feature"]);
    let feature = commit(cwd, "feature work");
    let executor = ExecutorManager::default();
    Repository {
        _directory: directory,
        queries: GitQueries::new(executor.clone()),
        executor,
        base,
        second,
        feature,
    }
}

impl Repository {
    fn path(&self) -> &Path {
        self._directory.path()
    }

    fn assert_no_process_left(&self) {
        assert_eq!(executor_registry_counts(&self.executor), (0, 0, 0));
    }
}

#[test]
fn the_branches_list_the_default_first_and_the_current_one_is_named() {
    let repository = repository();

    assert_eq!(
        repository
            .queries
            .local_branches(repository.path())
            .unwrap(),
        vec!["main", "alpha", "feature"]
    );
    assert_eq!(
        repository
            .queries
            .current_branch(repository.path())
            .unwrap(),
        Some("feature".to_string())
    );
    setup_git(repository.path(), &["checkout", "-q", "--detach"]);
    assert_eq!(
        repository
            .queries
            .current_branch(repository.path())
            .unwrap(),
        None
    );
    repository.assert_no_process_left();
}

#[test]
fn recent_commits_are_newest_first_and_bounded() {
    let repository = repository();

    assert_eq!(
        repository
            .queries
            .recent_commits(repository.path(), 10)
            .unwrap(),
        vec![
            GitCommit {
                sha: repository.feature.clone(),
                subject: "feature work".to_string(),
            },
            GitCommit {
                sha: repository.base.clone(),
                subject: "base".to_string(),
            },
        ]
    );
    assert_eq!(
        repository
            .queries
            .recent_commits(repository.path(), 1)
            .unwrap()
            .len(),
        1
    );
    repository.assert_no_process_left();
}

#[test]
fn a_repository_without_commits_has_no_recent_commit_and_no_merge_base() {
    let directory = tempfile::tempdir().unwrap();
    setup_git(directory.path(), &["init", "-q", "-b", "main"]);
    let queries = GitQueries::new(ExecutorManager::default());

    assert_eq!(queries.recent_commits(directory.path(), 10).unwrap(), []);
    assert_eq!(
        queries.local_branches(directory.path()).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        queries
            .merge_base_with_head(directory.path(), "main")
            .unwrap(),
        None
    );
}

#[test]
fn the_merge_base_is_where_head_left_the_branch() {
    let repository = repository();

    assert_eq!(
        repository
            .queries
            .merge_base_with_head(repository.path(), "main")
            .unwrap(),
        Some(repository.base.clone())
    );
    assert_ne!(repository.base, repository.second);
    assert_eq!(
        repository
            .queries
            .merge_base_with_head(repository.path(), "missing")
            .unwrap(),
        None,
        "a branch that does not exist has no merge base"
    );
    repository.assert_no_process_left();
}

#[test]
fn outside_a_repository_a_query_is_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let queries = GitQueries::new(ExecutorManager::default());

    assert!(matches!(
        queries.current_branch(directory.path()),
        Err(WorktreeError::Git(_))
    ));
}
