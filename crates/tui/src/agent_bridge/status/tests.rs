use std::fs;

use super::*;

#[test]
fn the_branch_is_read_from_head_in_the_directory_or_above() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join(".git")).unwrap();
    fs::write(repo.path().join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
    let nested = repo.path().join("src/deep");
    fs::create_dir_all(&nested).unwrap();
    assert_eq!(git_branch(&nested).as_deref(), Some("feature/x"));

    fs::write(
        repo.path().join(".git/HEAD"),
        "0123456789abcdef0123456789abcdef01234567\n",
    )
    .unwrap();
    assert_eq!(git_branch(repo.path()).as_deref(), Some("0123456"));
}

#[test]
fn a_worktree_reads_the_git_directory_its_git_file_names() {
    let root = tempfile::tempdir().unwrap();
    let git_dir = root.path().join("main/.git/worktrees/wt");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(git_dir.join("HEAD"), "ref: refs/heads/wt-branch\n").unwrap();
    let worktree = root.path().join("wt");
    fs::create_dir_all(&worktree).unwrap();
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", git_dir.display()),
    )
    .unwrap();
    assert_eq!(git_branch(&worktree).as_deref(), Some("wt-branch"));
}

#[test]
fn outside_a_repository_there_is_no_branch() {
    let directory = tempfile::tempdir().unwrap();
    // A `.git` directory with no HEAD is not a repository's.
    fs::create_dir_all(directory.path().join(".git")).unwrap();
    assert_eq!(git_branch(directory.path()), None);
}
