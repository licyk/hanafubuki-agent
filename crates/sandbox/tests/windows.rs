//! The Windows sandbox end to end: this binary answers the helper's
//! command line first, as the Agent's does, so the commands it confines
//! start it again as the helper. Without a harness, `main` runs each check
//! in turn.

#[cfg(windows)]
fn main() {
    hanafubuki_agent_sandbox::helper_dispatch();
    windows::run();
}

#[cfg(not(windows))]
fn main() {
    hanafubuki_agent_sandbox::helper_dispatch();
}

#[cfg(windows)]
mod windows {
    use std::path::Path;
    use std::process::{Command, Output};

    use hanafubuki_agent_sandbox::{
        AgentSandboxMode, AgentWindowsSandbox, Sandbox, SandboxPolicy, SandboxSettings,
        SandboxStatus,
    };

    pub(super) fn run() {
        let detected = Sandbox::detect();
        let off = detected.with_windows_sandbox(None);
        assert!(
            off.wrap(&SandboxPolicy::ReadOnly, cmd("echo")).is_err(),
            "an unset windows.sandbox must refuse"
        );
        let sandbox = detected.with_windows_sandbox(Some(AgentWindowsSandbox::Unelevated));
        assert!(
            matches!(sandbox.status(), SandboxStatus::Available { .. }),
            "{:?}",
            sandbox.status()
        );
        type Check = fn(&Sandbox);
        let checks: [(&str, Check); 6] = [
            (
                "read_only_cannot_write_the_workspace",
                read_only_cannot_write_the_workspace,
            ),
            (
                "workspace_write_writes_the_workspace_and_temp_and_nothing_outside",
                workspace_write_writes_the_workspace_and_temp_and_nothing_outside,
            ),
            (
                "the_repository_stays_read_only",
                the_repository_stays_read_only,
            ),
            (
                "without_the_network_a_command_gets_the_offline_environment",
                without_the_network_a_command_gets_the_offline_environment,
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
        if std::env::var_os("HANAFUBUKI_TEST_ELEVATED_SANDBOX").is_some() {
            hanafubuki_agent_sandbox::setup_windows_sandbox().unwrap();
            let elevated = detected.with_windows_sandbox(Some(AgentWindowsSandbox::Elevated));
            assert!(
                matches!(elevated.status(), SandboxStatus::Available { .. }),
                "{:?}",
                elevated.status()
            );
            for (name, check) in checks {
                check(&elevated);
                eprintln!("ok: elevated {name}");
            }
            elevated_network_is_enforced(&elevated);
            stdio_round_trip(&elevated);
        }
    }

    /// Directories outside `TEMP`, which workspace-write always allows.
    fn directory() -> tempfile::TempDir {
        tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
    }

    /// `script` run by `cmd.exe`. Paths in it are written unquoted, since
    /// `cmd` does not read the quotes the command line escapes; the test
    /// directories hold no spaces.
    fn cmd(script: &str) -> Vec<String> {
        let comspec =
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_string());
        vec![
            comspec,
            "/D".to_string(),
            "/S".to_string(),
            "/C".to_string(),
            script.to_string(),
        ]
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
            cmd(&format!("echo written>{}", file.display())),
        )
    }

    /// Workspace-write over `root`, as a turn's settings make it.
    fn workspace_write(root: &Path, network_access: bool) -> SandboxPolicy {
        SandboxSettings {
            mode: AgentSandboxMode::WorkspaceWrite,
            writable_roots: Vec::new(),
            network_access,
            windows_sandbox: Some(AgentWindowsSandbox::Unelevated),
        }
        .policy(&[root.to_path_buf()])
    }

    fn read_only_cannot_write_the_workspace(sandbox: &Sandbox) {
        let workspace = directory();
        std::fs::write(workspace.path().join("readable"), "content").unwrap();
        let read = run_confined(
            sandbox,
            &SandboxPolicy::ReadOnly,
            cmd(&format!(
                "type {}",
                workspace.path().join("readable").display()
            )),
        );
        assert!(read.status.success(), "{read:?}");
        assert_eq!(read.stdout, b"content");

        let file = workspace.path().join("file");
        let written = write(sandbox, &SandboxPolicy::ReadOnly, &file);
        assert!(!written.status.success(), "{written:?}");
        assert!(!file.exists());
        let null = run_confined(sandbox, &SandboxPolicy::ReadOnly, cmd("echo x>NUL"));
        assert!(null.status.success(), "{null:?}");
    }

    fn workspace_write_writes_the_workspace_and_temp_and_nothing_outside(sandbox: &Sandbox) {
        let workspace = directory();
        let outside = directory();
        let policy = workspace_write(workspace.path(), false);

        let nested = workspace.path().join("nested");
        let inside = run_confined(
            sandbox,
            &policy,
            cmd(&format!(
                "mkdir {} && echo written>{}",
                nested.display(),
                nested.join("file").display()
            )),
        );
        assert!(inside.status.success(), "{inside:?}");
        assert_eq!(
            std::fs::read_to_string(nested.join("file")).unwrap(),
            "written\r\n"
        );

        let escaped = outside.path().join("file");
        let written = write(sandbox, &policy, &escaped);
        assert!(!written.status.success(), "{written:?}");
        assert!(!escaped.exists());

        // Closed first: `cmd`'s redirection is refused a file another
        // process holds open (a sharing violation).
        let temporary = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let in_temp = write(sandbox, &policy, &temporary);
        assert!(in_temp.status.success(), "{in_temp:?}");
    }

    fn the_repository_stays_read_only(sandbox: &Sandbox) {
        let workspace = directory();
        std::fs::create_dir(workspace.path().join(".git")).unwrap();
        let hook = workspace.path().join(".git").join("hook");
        let written = write(sandbox, &workspace_write(workspace.path(), false), &hook);
        assert!(!written.status.success(), "{written:?}");
        assert!(!hook.exists());
        let removed = run_confined(
            sandbox,
            &workspace_write(workspace.path(), false),
            cmd(&format!(
                "rmdir {}",
                workspace.path().join(".git").display()
            )),
        );
        assert!(workspace.path().join(".git").exists(), "{removed:?}");
    }

    fn without_the_network_a_command_gets_the_offline_environment(sandbox: &Sandbox) {
        let workspace = directory();
        let echo = cmd("echo %SBX_NONET_ACTIVE%");
        let offline = run_confined(
            sandbox,
            &workspace_write(workspace.path(), false),
            echo.clone(),
        );
        assert_eq!(String::from_utf8_lossy(&offline.stdout).trim(), "1");
        let online = run_confined(sandbox, &workspace_write(workspace.path(), true), echo);
        assert_eq!(
            String::from_utf8_lossy(&online.stdout).trim(),
            "%SBX_NONET_ACTIVE%"
        );
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
                cmd(&format!("echo ran>{}", marker.display())),
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
    fn stdio_round_trip(sandbox: &Sandbox) {
        use std::io::Write;
        use std::process::Stdio;
        let workspace = directory();
        let command = sandbox
            .wrap(
                &workspace_write(workspace.path(), false),
                cmd("set /p VALUE=& call echo %%VALUE%%& echo diagnostic 1>&2"),
            )
            .unwrap();
        let mut child = Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"round-trip\r\n")
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("round-trip"),
            "{result:?}"
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("diagnostic"),
            "{result:?}"
        );
        eprintln!("ok: elevated stdio_round_trip");
    }

    fn elevated_network_is_enforced(sandbox: &Sandbox) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let workspace = directory();
        let powershell = std::env::var("SystemRoot").unwrap()
            + r"\System32\WindowsPowerShell\v1.0\powershell.exe";
        let script = format!("try {{ $c = New-Object Net.Sockets.TcpClient; $c.Connect('127.0.0.1', {port}); $c.Dispose(); exit 0 }} catch {{ exit 7 }}");
        let command = || {
            vec![
                powershell.clone(),
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                script.clone(),
            ]
        };
        let blocked = run_confined(
            sandbox,
            &workspace_write(workspace.path(), false),
            command(),
        );
        assert_eq!(blocked.status.code(), Some(7), "{blocked:?}");
        let allowed = run_confined(sandbox, &workspace_write(workspace.path(), true), command());
        assert!(allowed.status.success(), "{allowed:?}");
    }
}
