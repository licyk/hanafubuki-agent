use std::fs;
use std::process::Command;

use super::*;

fn git_in(directory: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(directory)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

#[test]
fn tracked_changes_and_untracked_files_are_both_in_the_diff() {
    let repository = tempfile::tempdir().unwrap();
    git_in(repository.path(), &["init", "--quiet"]);
    fs::write(repository.path().join("tracked.txt"), "before\n").unwrap();
    git_in(repository.path(), &["add", "tracked.txt"]);
    git_in(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    fs::write(repository.path().join("tracked.txt"), "after\n").unwrap();
    fs::write(repository.path().join("untracked.txt"), "new\n").unwrap();

    let (is_repository, diff) = get_git_diff(repository.path()).unwrap();

    assert!(is_repository);
    assert!(diff.contains("-before"), "{diff}");
    assert!(diff.contains("+after"), "{diff}");
    assert!(diff.contains("untracked.txt"), "{diff}");
    assert!(diff.contains("+new"), "{diff}");
}

#[test]
fn a_directory_outside_a_repository_has_no_diff() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        get_git_diff(directory.path()).unwrap(),
        (false, String::new())
    );
}
