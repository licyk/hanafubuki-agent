//! The package store against real directories and repositories: the
//! manifest formats, the cache, the marketplaces, the configuration's
//! enablement, and the lifecycle of Codex's, Claude Code's, Agent Plugins'
//! and this project's packages from a local and a git marketplace. The
//! fixture packages are in `fixtures/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hanafubuki_agent_packages::PackageStore;
use hanafubuki_agent_process::executor::manager::executor_registry_counts;
use hanafubuki_agent_process::executor::ExecutorManager;

mod lifecycle;
mod link;
mod manifest;
mod marketplace;
mod options;
mod probe;
mod settings;
mod store;

/// A store under a temporary data directory, and the directory.
struct Fixture {
    directory: tempfile::TempDir,
    executor: ExecutorManager,
    store: PackageStore,
}

fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let executor = ExecutorManager::default();
    let store = PackageStore::open(directory.path().join("plugins"), executor.clone());
    Fixture {
        directory,
        executor,
        store,
    }
}

impl Fixture {
    fn path(&self) -> &Path {
        self.directory.path()
    }

    fn plugins(&self) -> PathBuf {
        self.path().join("plugins")
    }

    /// Every `git` the store ran was disposed of.
    fn assert_no_process_left(&self) {
        assert_eq!(executor_registry_counts(&self.executor), (0, 0, 0));
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/packages/fixtures")
}

/// Writes `contents` at `path`, making its directories.
fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A Codex package at `root/dir_name` named `name`, with a Skill and an
/// `.mcp.json`.
fn write_plugin(root: &Path, dir_name: &str, name: &str, version: Option<&str>) -> PathBuf {
    let package = root.join(dir_name);
    let version = version
        .map(|version| format!(r#","version":"{version}""#))
        .unwrap_or_default();
    write(
        &package.join(".codex-plugin/plugin.json"),
        &format!(r#"{{"name":"{name}"{version}}}"#),
    );
    write(&package.join("skills/SKILL.md"), "skill");
    write(&package.join(".mcp.json"), r#"{"mcpServers":{}}"#);
    package
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

/// A bare repository at `bare` holding one commit of `contents`, and its
/// `file://` URL.
fn bare_repository(contents: &Path, work: &Path, bare: &Path) -> String {
    copy_dir(contents, work);
    setup_git(work, &["init", "-q", "-b", "main"]);
    setup_git(work, &["config", "core.autocrlf", "false"]);
    commit(work, "initial");
    fs::create_dir_all(bare.parent().unwrap()).unwrap();
    setup_git(
        bare.parent().unwrap(),
        &[
            "clone",
            "-q",
            "--bare",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    file_url(bare)
}

fn commit(work: &Path, message: &str) {
    setup_git(work, &["add", "-A"]);
    setup_git(
        work,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
}

fn file_url(path: &Path) -> String {
    let path = path.to_str().unwrap().replace('\\', "/");
    format!("file:///{}", path.trim_start_matches('/'))
}
