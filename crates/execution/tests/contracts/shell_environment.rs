use super::*;
use hanafubuki_agent_execution::ExecutionEnvironmentContext;
#[cfg(unix)]
use hanafubuki_agent_execution::ShellEnvironment;
use hanafubuki_agent_execution::{ExecCommandRequest, PlatformShell};
#[cfg(unix)]
use hanafubuki_agent_foundation::environment::ChildProcessEnvironment;
use hanafubuki_agent_foundation::environment_policy::EnvironmentInherit;
#[cfg(unix)]
use hanafubuki_agent_foundation::environment_policy::EnvironmentPolicy;
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_sandbox::SandboxSettings;

#[cfg(unix)]
#[test]
fn snapshot_reuses_startup_and_filters_exports_for_real_shells() {
    const MARKER: &str = "HANAFUBUKI_SNAPSHOT_TEST";
    if std::env::var_os(MARKER).is_none() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("profile");
        std::fs::write(
            &profile,
            r#"
printf x >> "$HANAFUBUKI_SNAPSHOT_TEST/count"
export SNAPSHOT_VALUE='profile value'
export SNAPSHOT_SECRET='fake secret'
export SNAPSHOT_MULTILINE='line one
line two'
hb_test_function() { printf 'function-ok\n'; }
alias hb_test_alias='printf "alias-ok\n"'
if [ -n "${BASH_VERSION-}" ]; then shopt -s expand_aliases; fi
"#,
        )
        .unwrap();
        std::fs::copy(&profile, temp.path().join(".zshrc")).unwrap();
        for shell in ["/bin/bash", "/bin/zsh"] {
            if !std::path::Path::new(shell).is_file() {
                continue;
            }
            let _ = std::fs::remove_file(temp.path().join("count"));
            let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "shell_environment::snapshot_reuses_startup_and_filters_exports_for_real_shells",
                "--nocapture",
            ])
            .env(MARKER, temp.path())
            // Login shells also read HOME's profile, independently of BASH_ENV.
            .env("HOME", temp.path())
            .env("BASH_ENV", &profile)
            .env("ZDOTDIR", temp.path())
            .env("SHELL", shell)
            .env("HTTP_PROXY", "http://inherited.invalid:8080")
            .output()
            .unwrap();
            assert!(
                output.status.success(),
                "{shell}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    }
    let overlay = ChildProcessEnvironment::default();
    let service = AgentExecutionService::new(ExecutorManager::new(overlay.clone()));
    let mut request = exec_request(false);
    request.shell_environment = ShellEnvironment {
        policy: EnvironmentPolicy {
            ignore_default_excludes: false,
            ..Default::default()
        },
        context: Some(ExecutionEnvironmentContext {
            session_id: 301,
            turn_id: 7,
            tool_call_id: "call-one".into(),
            permission_mode: Some("full_access".into()),
        }),
        ..Default::default()
    };
    request.cmd = "hb_test_function; hb_test_alias; printf '%s\\n' \"$SNAPSHOT_VALUE\" \"$SNAPSHOT_MULTILINE\" \"secret=${SNAPSHOT_SECRET-unset}\" \"$HANAFUBUKI_AGENT_TOOL_CALL_ID\" \"$CODEX_CI:$PYTHONUTF8\"".into();
    for tty in [false, true] {
        request.tty = tty;
        let output = run(&service, request.clone());
        for text in [
            "function-ok",
            "alias-ok",
            "profile value",
            "line one",
            "line two",
            "secret=unset",
            "call-one",
            "1:1",
        ] {
            assert!(output.contains(text), "missing {text}: {output}");
        }
    }
    let count = std::path::PathBuf::from(std::env::var_os(MARKER).unwrap()).join("count");
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "x");
    request
        .shell_environment
        .context
        .as_mut()
        .unwrap()
        .tool_call_id = "call-two".into();
    request.shell_environment.policy.inherit = EnvironmentInherit::Unfiltered;
    request.shell_environment.policy.exclude = vec!["*".into()];
    request.shell_environment.policy.include_only = vec!["NOT_A_VARIABLE".into()];
    request.cmd = "printf '%s\\n' \"$HANAFUBUKI_AGENT_TOOL_CALL_ID\" \"$HTTP_PROXY\" \"$SNAPSHOT_SECRET\" \"$CODEX_CI:$PYTHONUTF8\"".into();
    overlay.replace(std::collections::HashMap::from([(
        "HTTP_PROXY".into(),
        Some("http://configured.invalid:9000".into()),
    )]));
    assert!(run(&service, request.clone()).contains("http://configured.invalid:9000"));
    overlay.replace(Default::default());
    let output = run(&service, request);
    assert!(output.contains("call-two"));
    assert!(output.contains("fake secret"), "{output}");
    assert!(output.contains("1:1"), "{output}");
    assert!(output.contains("http://inherited.invalid:8080"), "{output}");
    assert_eq!(std::fs::read_to_string(count).unwrap(), "xxx");
    assert_eq!(service.registry_count(), 0);
}

#[test]
fn empty_inheritance_still_has_defaults_overrides_and_runtime_identity() {
    let service = AgentExecutionService::new(ExecutorManager::default());
    let mut request = if cfg!(windows) {
        ExecCommandRequest {
            cmd: "[Console]::WriteLine($env:HB_TEST_VALUE + ':' + $env:HANAFUBUKI_AGENT_SESSION_ID + ':' + $env:CODEX_CI + ':' + $env:PYTHONUTF8 + ':' + $env:PATHEXT + ':' + $env:SYSTEMROOT)".into(),
            shell: Some(PlatformShell::PowerShell), login: false, tty: false,
            yield_time_ms: 1000, timeout_ms: 30_000, max_output_bytes: Some(4096),
            workdir: Some(std::env::current_dir().unwrap()), environment: Default::default(),
            shell_environment: Default::default(), origin: ApprovalOrigin::Model,
            sandbox: SandboxSettings::full_access(), sandbox_escalation: None,
        }
    } else {
        #[cfg(unix)]
        {
            let mut request = exec_request(false);
            request.login = false;
            request.cmd = "printf '%s\\n' \"$HB_TEST_VALUE:$HANAFUBUKI_AGENT_SESSION_ID:$CODEX_CI:$PYTHONUTF8\"".into();
            request
        }
        #[cfg(not(unix))]
        {
            return;
        }
    };
    request.shell_environment.policy.inherit = EnvironmentInherit::None;
    request
        .shell_environment
        .policy
        .set
        .insert("HB_TEST_VALUE".into(), "configured".into());
    request
        .environment
        .insert("HB_TEST_VALUE".into(), "request".into());
    request
        .environment
        .insert("HANAFUBUKI_AGENT_SESSION_ID".into(), "spoofed".into());
    request.shell_environment.context = Some(ExecutionEnvironmentContext {
        session_id: 301,
        turn_id: 8,
        tool_call_id: "call".into(),
        permission_mode: None,
    });
    let output = run(&service, request);
    assert!(output.contains("request:301:1:1"), "{output}");
    if cfg!(windows) {
        assert!(output.contains(".COM;.EXE;.BAT;.CMD"), "{output}");
        assert!(
            output.contains(&std::env::var("SystemRoot").unwrap()),
            "{output}"
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_hidden_environment_survives_shell_execution() {
    use hanafubuki_agent_foundation::paths::ResolvedPath;

    const MARKER: &str = "HANAFUBUKI_HIDDEN_ENV_TEST";
    const TEST: &str = "shell_environment::windows_hidden_environment_survives_shell_execution";
    const VALUE: &str = r"Z:\hanafubuki test";
    let executable = ResolvedPath::resolve(std::env::current_exe().unwrap()).unwrap();
    match std::env::var(MARKER).as_deref() {
        Ok("probe") => {
            assert_eq!(std::env::var("=Z:").unwrap(), VALUE);
            println!("hidden-environment-preserved");
            return;
        }
        Ok("runner") => {}
        _ => {
            // Give only the isolated runner a hidden drive entry; never mutate
            // the test harness environment shared by concurrently running tests.
            let output = std::process::Command::new(executable.system())
                .args(["--exact", TEST, "--nocapture"])
                .env(MARKER, "runner")
                .env("=Z:", VALUE)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
    }
    assert_eq!(std::env::var("=Z:").unwrap(), VALUE);
    let service = AgentExecutionService::new(ExecutorManager::default());
    for inherit in [EnvironmentInherit::All, EnvironmentInherit::Unfiltered] {
        for shell in [PlatformShell::PowerShell, PlatformShell::Cmd] {
            let cmd = if shell == PlatformShell::PowerShell {
                format!(
                    "& '{}' --exact {TEST} --nocapture; exit $LASTEXITCODE",
                    executable.native_text().replace('\'', "''")
                )
            } else {
                // Run by basename from its directory so this environment test
                // does not depend on CMD's quoting of absolute paths.
                format!(
                    "{} --exact {TEST} --nocapture",
                    executable.file_name().unwrap().to_str().unwrap()
                )
            };
            for tty in [false, true] {
                let mut request = ExecCommandRequest {
                    cmd: cmd.clone(),
                    shell: Some(shell),
                    login: false,
                    tty,
                    yield_time_ms: 1000,
                    timeout_ms: 30_000,
                    max_output_bytes: Some(4096),
                    workdir: Some(executable.system().parent().unwrap().to_path_buf()),
                    environment: BTreeMap::from([(MARKER.into(), "probe".into())]),
                    shell_environment: Default::default(),
                    origin: ApprovalOrigin::Model,
                    sandbox: SandboxSettings::full_access(),
                    sandbox_escalation: None,
                };
                request.shell_environment.policy.inherit = inherit;
                let output = run(&service, request);
                assert!(output.contains("hidden-environment-preserved"), "{output}");
            }
        }
    }
}

fn run(service: &AgentExecutionService, request: ExecCommandRequest) -> String {
    let scope = AgentExecutionScope::for_agent_session(301).unwrap();
    let action = service.prepare_exec_command_action(&request).unwrap();
    let cancellation = CancellationToken::default();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &cancellation,
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(45);
    let mut output = String::new();
    loop {
        assert!(std::time::Instant::now() < deadline, "shell did not finish");
        let result = service
            .wait_unified_execution(
                &scope,
                started.execution_id,
                1000,
                Some(32 * 1024),
                &cancellation,
            )
            .unwrap();
        output.push_str(&result.output);
        if result.session_id.is_none() {
            assert_eq!(result.exit_code, Some(0), "{output}");
            return output;
        }
    }
}
