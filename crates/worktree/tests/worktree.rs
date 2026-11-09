//! The worktree manager against a real repository: create, list, bind and
//! remove, each `git` through the executor and disposed of.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_process::executor::manager::executor_registry_counts;
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_worktree::{Worktree, WorktreeError, WorktreeManager};

struct Fixture {
    _directory: tempfile::TempDir,
    repository: PathBuf,
    root: PathBuf,
    executor: ExecutorManager,
    worktrees: WorktreeManager,
}

/// A repository with one commit of `file.txt` and a `src` directory, and a
/// manager whose worktrees go under `data/worktrees`.
fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("project");
    fs::create_dir_all(repository.join("src")).unwrap();
    fs::write(repository.join("file.txt"), "committed\n").unwrap();
    fs::write(repository.join("src/lib.rs"), "").unwrap();
    setup_git(&repository, &["init", "-q", "-b", "main"]);
    setup_git(&repository, &["add", "."]);
    setup_git(
        &repository,
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
    let root = directory.path().join("data").join("worktrees");
    let executor = ExecutorManager::default();
    Fixture {
        worktrees: WorktreeManager::new(root.clone(), executor.clone()),
        _directory: directory,
        repository,
        root,
        executor,
    }
}

fn setup_git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn native(path: &Path) -> PathBuf {
    ResolvedPath::resolve(path).unwrap().native().to_path_buf()
}

impl Fixture {
    fn assert_no_process_left(&self) {
        assert_eq!(executor_registry_counts(&self.executor), (0, 0, 0));
    }
}

#[test]
fn a_worktree_is_a_detached_checkout_of_head_named_after_its_repository() {
    let fixture = fixture();
    let head = setup_git(&fixture.repository, &["rev-parse", "HEAD"]);
    // Uncommitted work stays in the source checkout.
    fs::write(fixture.repository.join("file.txt"), "uncommitted\n").unwrap();

    let created = fixture
        .worktrees
        .create(&fixture.repository.join("src"))
        .unwrap();

    assert_eq!(created.head, head);
    assert_eq!(created.branch, None);
    assert_eq!(created.owner, None);
    assert_eq!(created.path.parent(), Some(native(&fixture.root).as_path()));
    let name = created.path.file_name().unwrap().to_str().unwrap();
    let id = name.strip_prefix("project-").unwrap();
    assert_eq!(id.len(), 8, "{name}");
    assert_eq!(
        fs::read_to_string(created.path.join("file.txt")).unwrap(),
        "committed\n"
    );
    assert!(created.path.join("src/lib.rs").is_file());
    assert_eq!(setup_git(&created.path, &["status", "--porcelain"]), "");
    fixture.assert_no_process_left();
}

#[test]
fn the_repositorys_worktrees_are_listed_with_their_owner_from_any_checkout_of_it() {
    let fixture = fixture();
    let first = fixture.worktrees.create(&fixture.repository).unwrap();
    let second = fixture.worktrees.create(&fixture.repository).unwrap();

    fixture.worktrees.bind(&first.path, "7").unwrap();
    // The same session again changes nothing; another is refused.
    fixture.worktrees.bind(&first.path, "7").unwrap();
    assert_eq!(
        fixture.worktrees.bind(&first.path, "8"),
        Err(WorktreeError::Owned("7".to_string()))
    );
    // A branch checked out in a worktree is listed.
    setup_git(&second.path, &["switch", "-q", "-c", "feature"]);

    let mut expected = vec![
        Worktree {
            owner: Some("7".to_string()),
            ..first.clone()
        },
        Worktree {
            branch: Some("feature".to_string()),
            ..second.clone()
        },
    ];
    expected.sort_by(|left, right| left.path.cmp(&right.path));
    // The source checkout is not one of them.
    assert_eq!(
        fixture.worktrees.list(&fixture.repository).unwrap(),
        expected
    );
    assert_eq!(fixture.worktrees.list(&second.path).unwrap(), expected);
    fixture.assert_no_process_left();
}

#[test]
fn an_owner_record_that_does_not_read_is_quarantined() {
    let fixture = fixture();
    let created = fixture.worktrees.create(&fixture.repository).unwrap();
    let record = PathBuf::from(setup_git(
        &created.path,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "hanafubuki-agent-owner.json",
        ],
    ));
    fs::write(&record, "{not json").unwrap();

    let listed = fixture.worktrees.list(&fixture.repository).unwrap();

    assert_eq!(listed[0].owner, None);
    assert!(!record.exists());
    let recovery = record.parent().unwrap().join(".recovery");
    assert_eq!(fs::read_dir(recovery).unwrap().count(), 1);
    fixture.worktrees.bind(&created.path, "3").unwrap();
    assert_eq!(
        fixture.worktrees.list(&fixture.repository).unwrap()[0].owner,
        Some("3".to_string())
    );
}

#[test]
fn a_worktree_is_removed_with_its_changes_but_never_from_inside_it() {
    let fixture = fixture();
    let created = fixture.worktrees.create(&fixture.repository).unwrap();
    fixture.worktrees.bind(&created.path, "7").unwrap();
    fs::write(created.path.join("file.txt"), "changed\n").unwrap();
    fs::write(created.path.join("new.txt"), "untracked\n").unwrap();

    assert_eq!(
        fixture
            .worktrees
            .remove(&created.path.join("src"), &created.path),
        Err(WorktreeError::InUse(created.path.clone()))
    );
    fixture
        .worktrees
        .remove(&fixture.repository, &created.path)
        .unwrap();

    assert!(!created.path.exists());
    assert_eq!(fixture.worktrees.list(&fixture.repository).unwrap(), []);
    assert!(!setup_git(&fixture.repository, &["worktree", "list"])
        .contains(created.path.to_str().unwrap()));
    fixture.assert_no_process_left();
}

#[test]
fn only_a_worktree_directly_under_the_managed_directory_is_bound_or_removed() {
    let fixture = fixture();
    let outside = fixture.root.parent().unwrap().join("outside");
    setup_git(
        &fixture.repository,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            outside.to_str().unwrap(),
        ],
    );

    for path in [fixture.repository.clone(), outside.clone()] {
        assert_eq!(
            fixture.worktrees.bind(&path, "1"),
            Err(WorktreeError::NotManaged(path.clone()))
        );
        assert_eq!(
            fixture.worktrees.remove(&fixture.repository, &path),
            Err(WorktreeError::NotManaged(path.clone()))
        );
    }
    assert!(outside.is_dir());
    assert_eq!(fixture.worktrees.list(&fixture.repository).unwrap(), []);
}

#[test]
fn a_directory_outside_any_repository_has_no_worktree() {
    let fixture = fixture();
    let plain = fixture.root.parent().unwrap().join("plain");
    fs::create_dir_all(&plain).unwrap();

    let error = fixture.worktrees.create(&plain).unwrap_err();

    assert!(
        matches!(&error, WorktreeError::Git(message) if message.contains("not a git repository")),
        "{error}"
    );
    fixture.assert_no_process_left();
}
