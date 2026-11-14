//! The macOS sandbox end to end: commands run under `sandbox-exec` with the
//! policy `Sandbox::wrap` writes, as the Agent runs them.

#![cfg(target_os = "macos")]

use std::io::Read;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use hanafubuki_agent_sandbox::{
    AgentSandboxMode, Sandbox, SandboxPolicy, SandboxSettings, SandboxStatus,
};

fn sandbox() -> Sandbox {
    let sandbox = Sandbox::detect();
    assert!(
        matches!(sandbox.status(), SandboxStatus::Available { .. }),
        "{:?}",
        sandbox.status()
    );
    sandbox
}

/// Directories outside `/tmp` and `TMPDIR`, which workspace-write always
/// allows.
fn directory() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
}

fn sh(script: &str) -> Vec<String> {
    vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()]
}

fn run_confined(policy: &SandboxPolicy, command: Vec<String>) -> Output {
    let command = sandbox().wrap(policy, command).unwrap();
    Command::new(&command[0])
        .args(&command[1..])
        .output()
        .unwrap()
}

fn write(policy: &SandboxPolicy, file: &Path) -> Output {
    run_confined(policy, sh(&format!("echo written > '{}'", file.display())))
}

/// Workspace-write over `root`, as a turn's settings make it.
fn workspace_write(root: &Path, network_access: bool) -> SandboxPolicy {
    SandboxSettings {
        mode: AgentSandboxMode::WorkspaceWrite,
        writable_roots: Vec::new(),
        network_access,
        windows_sandbox: None,
    }
    .policy(&[root.to_path_buf()])
}

#[test]
fn read_only_cannot_write_the_workspace() {
    let workspace = directory();
    std::fs::write(workspace.path().join("readable"), "content").unwrap();
    let read = run_confined(
        &SandboxPolicy::ReadOnly,
        sh(&format!(
            "cat '{}'",
            workspace.path().join("readable").display()
        )),
    );
    assert!(read.status.success(), "{read:?}");
    assert_eq!(read.stdout, b"content");

    let file = workspace.path().join("file");
    let written = write(&SandboxPolicy::ReadOnly, &file);
    assert!(!written.status.success(), "{written:?}");
    assert!(!file.exists());
    let dev_null = run_confined(&SandboxPolicy::ReadOnly, sh("echo x > /dev/null"));
    assert!(dev_null.status.success(), "{dev_null:?}");
}

#[test]
fn workspace_write_writes_the_workspace_and_nothing_outside() {
    let workspace = directory();
    let outside = directory();
    let policy = workspace_write(workspace.path(), false);

    let nested = workspace.path().join("nested/file");
    let inside = run_confined(
        &policy,
        sh(&format!(
            "mkdir -p '{}' && echo written > '{}'",
            nested.parent().unwrap().display(),
            nested.display()
        )),
    );
    assert!(inside.status.success(), "{inside:?}");
    assert_eq!(std::fs::read_to_string(&nested).unwrap(), "written\n");

    let escaped = outside.path().join("file");
    let written = write(&policy, &escaped);
    assert!(!written.status.success(), "{written:?}");
    assert!(!escaped.exists());

    // `/tmp` is a symlink to `/private/tmp`, which the policy names.
    let temporary = tempfile::NamedTempFile::new_in("/tmp").unwrap();
    let in_tmp = write(&policy, temporary.path());
    assert!(in_tmp.status.success(), "{in_tmp:?}");
}

#[test]
fn the_repository_under_a_writable_root_stays_read_only() {
    let workspace = directory();
    std::fs::create_dir(workspace.path().join(".git")).unwrap();
    let policy = workspace_write(workspace.path(), false);

    let config = workspace.path().join(".git/config");
    let written = write(&policy, &config);
    assert!(!written.status.success(), "{written:?}");
    assert!(!config.exists());

    let renamed = run_confined(
        &policy,
        sh(&format!(
            "mv '{}' '{}'",
            workspace.path().join(".git").display(),
            workspace.path().join("moved").display()
        )),
    );
    assert!(!renamed.status.success(), "{renamed:?}");
    assert!(workspace.path().join(".git").is_dir());

    let beside = workspace.path().join("file");
    let written = write(&policy, &beside);
    assert!(written.status.success(), "{written:?}");
}

/// Connects to a listener on the loopback through bash's `/dev/tcp`.
fn connects(policy: &SandboxPolicy) -> bool {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let command = vec![
        "/bin/bash".to_string(),
        "-c".to_string(),
        format!("exec 3<>/dev/tcp/127.0.0.1/{port} && echo connected >&3"),
    ];
    let output = run_confined(policy, command);
    let mut accepted = false;
    for _ in 0..50 {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut text = String::new();
            stream.set_nonblocking(false).unwrap();
            stream.read_to_string(&mut text).unwrap();
            accepted = text == "connected\n";
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(output.status.success(), accepted, "{output:?}");
    accepted
}

#[test]
fn network_is_blocked_unless_allowed() {
    let workspace = directory();
    assert!(!connects(&SandboxPolicy::ReadOnly));
    assert!(!connects(&workspace_write(workspace.path(), false)));
    assert!(connects(&workspace_write(workspace.path(), true)));
    assert!(connects(&SandboxPolicy::FullAccess));
}

#[test]
fn full_access_is_unrestricted() {
    let outside = directory();
    let file = outside.path().join("file");
    let written = write(&SandboxPolicy::FullAccess, &file);
    assert!(written.status.success(), "{written:?}");
    assert!(file.exists());
}

#[test]
fn overlapping_writable_roots_do_not_regrant_metadata() {
    let workspace = directory();
    let git = workspace.path().join(".git");
    std::fs::create_dir(&git).unwrap();
    let policy = SandboxPolicy::WorkspaceWrite {
        writable_roots: vec![workspace.path().to_path_buf(), git.clone()],
        network_access: false,
    };
    let result = write(&policy, &git.join("hook"));
    assert!(!result.status.success(), "{result:?}");
    assert!(!git.join("hook").exists());
    let result = run_confined(&policy, sh(&format!("rm -rf '{}'", git.display())));
    assert!(!result.status.success(), "{result:?}");
    assert!(git.exists());
}
