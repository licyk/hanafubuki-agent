use std::path::PathBuf;

use super::*;

fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn options(args: &[&str]) -> Options {
    Options::parse(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>()).unwrap()
}

#[test]
fn names_the_workspace_version_by_default() {
    let (tag, message) = tag_for(version("0.1.2"), &options(&[])).unwrap();
    assert_eq!(
        (tag.as_str(), message.as_str()),
        ("v0.1.2", "hanafubuki-agent 0.1.2")
    );
}

#[test]
fn accepts_the_workspace_version_with_or_without_v() {
    for given in ["0.1.2", "v0.1.2"] {
        let (tag, _) = tag_for(version("0.1.2"), &options(&["--version", given])).unwrap();
        assert_eq!(tag, "v0.1.2");
    }
}

#[test]
fn refuses_a_version_that_is_not_the_workspace_version() {
    let error = tag_for(version("0.1.2"), &options(&["--version", "0.1.3"])).unwrap_err();
    assert!(
        error.contains("not the workspace version 0.1.2") && error.contains("cargo xtask version"),
        "{error}"
    );
}

#[test]
fn takes_any_name_and_message() {
    let (tag, message) = tag_for(
        version("0.1.2"),
        &options(&["--name", "docs-1", "-m", "Docs"]),
    )
    .unwrap();
    assert_eq!((tag.as_str(), message.as_str()), ("docs-1", "Docs"));
    assert!(tag_for(version("0.1.2"), &options(&["--name", "two words"])).is_err());
    assert!(Options::parse(&[
        "--version".into(),
        "1.0.0".into(),
        "--name".into(),
        "x".into()
    ])
    .is_err());
}

#[test]
fn tags_and_pushes_in_a_real_repository() {
    let (repo, remote) = repository("push");
    let opts = Options {
        push: true,
        ..Options::default()
    };
    run(&repo, &opts).unwrap();
    assert_eq!(git(&repo, &["tag", "--list"]).unwrap().trim(), "v0.3.1");
    assert_eq!(
        git(&repo, &["cat-file", "-t", "v0.3.1"]).unwrap().trim(),
        "tag",
        "annotated"
    );
    assert!(git(&remote, &["tag", "--list"]).unwrap().contains("v0.3.1"));

    // the same tag again is refused
    let error = run(&repo, &Options::default()).unwrap_err();
    assert!(error.contains("already exists"), "{error}");
}

#[test]
fn refuses_a_dirty_tree_and_dry_runs_change_nothing() {
    let (repo, _) = repository("dirty");
    let dry_run = Options {
        dry_run: true,
        ..Options::default()
    };
    run(&repo, &dry_run).unwrap();
    assert_eq!(git(&repo, &["tag", "--list"]).unwrap().trim(), "");

    std::fs::write(repo.join("Cargo.toml"), MANIFEST.replace("0.3.1", "0.3.2")).unwrap();
    let error = run(&repo, &Options::default()).unwrap_err();
    assert!(error.contains("uncommitted"), "{error}");
}

const MANIFEST: &str = "[workspace]\n\n[workspace.package]\nversion = \"0.3.1\"\n";

/// A repository with one commit, and a bare remote named origin.
fn repository(name: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("xtask-tag-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (repo, remote) = (base.join("repo"), base.join("remote.git"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "--quiet", "--bare"]).unwrap();
    git(&repo, &["init", "--quiet"]).unwrap();
    for (key, value) in [
        ("user.name", "xtask"),
        ("user.email", "xtask@example.com"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
    ] {
        git(&repo, &["config", key, value]).unwrap();
    }
    std::fs::write(repo.join("Cargo.toml"), MANIFEST).unwrap();
    git(&repo, &["add", "."]).unwrap();
    git(&repo, &["commit", "--quiet", "-m", "init"]).unwrap();
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    )
    .unwrap();
    (repo, remote)
}
