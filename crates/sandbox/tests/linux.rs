//! The Linux sandbox end to end: this binary answers the helper's command
//! line first, as the Agent's does, so the commands it confines re-execute
//! it. Without a harness, `main` runs each check in turn; on a machine
//! without Landlock it says so and checks nothing but the refusal.

#[cfg(target_os = "linux")]
fn main() {
    hanafubuki_agent_sandbox::helper_dispatch();
    linux::run();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    hanafubuki_agent_sandbox::helper_dispatch();
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io::Read;
    use std::net::TcpListener;
    use std::path::Path;
    use std::process::{Command, Output};
    use std::time::Duration;

    use hanafubuki_agent_sandbox::{
        AgentSandboxMode, Sandbox, SandboxPolicy, SandboxSettings, SandboxStatus,
    };

    pub(super) fn run() {
        let sandbox = Sandbox::detect();
        if let SandboxStatus::Unavailable { reason } = sandbox.status() {
            eprintln!("skipped: the Linux sandbox is unavailable here ({reason})");
            let refused = sandbox.wrap(&SandboxPolicy::ReadOnly, sh("true"));
            assert!(refused.is_err(), "an unavailable sandbox must refuse");
            return;
        }
        type Check = fn(&Sandbox);
        let checks: [(&str, Check); 5] = [
            (
                "read_only_cannot_write_the_workspace",
                read_only_cannot_write_the_workspace,
            ),
            (
                "workspace_write_writes_the_workspace_and_nothing_outside",
                workspace_write_writes_the_workspace_and_nothing_outside,
            ),
            (
                "network_is_blocked_unless_allowed",
                network_is_blocked_unless_allowed,
            ),
            ("full_access_is_unrestricted", full_access_is_unrestricted),
            (
                "a_failed_confinement_runs_nothing",
                a_failed_confinement_runs_nothing,
            ),
        ];
        for (name, check) in checks {
            check(&sandbox);
            eprintln!("ok: {name}");
        }
    }

    /// Directories outside `/tmp` and `TMPDIR`, which workspace-write
    /// always allows.
    fn directory() -> tempfile::TempDir {
        tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
    }

    fn sh(script: &str) -> Vec<String> {
        vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()]
    }

    fn run_confined(sandbox: &Sandbox, policy: &SandboxPolicy, command: Vec<String>) -> Output {
        let command = sandbox.wrap(policy, command).unwrap();
        Command::new(&command[0])
            .args(&command[1..])
            .output()
            .unwrap()
    }

    fn write(sandbox: &Sandbox, policy: &SandboxPolicy, file: &Path) -> Output {
        run_confined(
            sandbox,
            policy,
            sh(&format!("echo written > '{}'", file.display())),
        )
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

    fn read_only_cannot_write_the_workspace(sandbox: &Sandbox) {
        let workspace = directory();
        std::fs::write(workspace.path().join("readable"), "content").unwrap();
        let read = run_confined(
            sandbox,
            &SandboxPolicy::ReadOnly,
            sh(&format!(
                "cat '{}'",
                workspace.path().join("readable").display()
            )),
        );
        assert!(read.status.success(), "{read:?}");
        assert_eq!(read.stdout, b"content");

        let file = workspace.path().join("file");
        let written = write(sandbox, &SandboxPolicy::ReadOnly, &file);
        assert!(!written.status.success(), "{written:?}");
        assert!(!file.exists());
        let dev_null = run_confined(sandbox, &SandboxPolicy::ReadOnly, sh("echo x > /dev/null"));
        assert!(dev_null.status.success(), "{dev_null:?}");
    }

    fn workspace_write_writes_the_workspace_and_nothing_outside(sandbox: &Sandbox) {
        let workspace = directory();
        let outside = directory();
        let policy = workspace_write(workspace.path(), false);

        let nested = workspace.path().join("nested/file");
        let inside = run_confined(
            sandbox,
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
        let written = write(sandbox, &policy, &escaped);
        assert!(!written.status.success(), "{written:?}");
        assert!(!escaped.exists());

        let temporary = tempfile::NamedTempFile::new_in("/tmp").unwrap();
        let in_tmp = write(sandbox, &policy, temporary.path());
        assert!(in_tmp.status.success(), "{in_tmp:?}");
    }

    /// Connects to a listener on the loopback through bash's `/dev/tcp`.
    fn connects(sandbox: &Sandbox, policy: &SandboxPolicy) -> bool {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let command = vec![
            "/bin/bash".to_string(),
            "-c".to_string(),
            format!("exec 3<>/dev/tcp/127.0.0.1/{port} && echo connected >&3"),
        ];
        let output = run_confined(sandbox, policy, command);
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

    fn network_is_blocked_unless_allowed(sandbox: &Sandbox) {
        if !Path::new("/bin/bash").exists() {
            eprintln!("skipped: the network check needs /bin/bash");
            return;
        }
        let workspace = directory();
        assert!(!connects(sandbox, &SandboxPolicy::ReadOnly));
        assert!(!connects(
            sandbox,
            &workspace_write(workspace.path(), false)
        ));
        assert!(connects(sandbox, &workspace_write(workspace.path(), true)));
        assert!(connects(sandbox, &SandboxPolicy::FullAccess));
    }

    fn full_access_is_unrestricted(sandbox: &Sandbox) {
        let outside = directory();
        let file = outside.path().join("file");
        let written = write(sandbox, &SandboxPolicy::FullAccess, &file);
        assert!(written.status.success(), "{written:?}");
        assert!(file.exists());
    }

    /// A helper that cannot confine the command (here, a command line it
    /// does not know) exits before the command runs.
    fn a_failed_confinement_runs_nothing(sandbox: &Sandbox) {
        let workspace = directory();
        let marker = workspace.path().join("ran");
        let mut command = sandbox
            .wrap(
                &workspace_write(workspace.path(), false),
                sh(&format!("echo ran > '{}'", marker.display())),
            )
            .unwrap();
        command.insert(2, "--writable-everything".to_string());
        let output = Command::new(&command[0])
            .args(&command[1..])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(hanafubuki_agent_sandbox::HELPER_FAILURE_EXIT_CODE),
            "{output:?}"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("hanafubuki-agent sandbox:"));
        assert!(!marker.exists());
    }
}
