use std::collections::{BTreeMap, HashMap};
use std::process::Command;
use std::time::{Duration, Instant};

use hanafubuki_agent_approval::ApprovalOrigin;
use hanafubuki_agent_execution::{
    AgentExecutionScope, AgentExecutionService, ExecCommandRequest, PlatformShell,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::environment::ChildProcessEnvironment;
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_sandbox::SandboxSettings;

use super::approve_action;

const CHILD: &str = "HANAFUBUKI_EXEC_ENV_TEST_CHILD";
const CUSTOM: &str = "HANAFUBUKI_EXEC_ENV_TEST_TOKEN";
const INHERITED: &str = "http://inherited.invalid:8080";
const EXPLICIT: &str = "http://configured.invalid:9000";
const REQUEST: &str = "http://request.invalid:8000";

#[test]
fn shell_processes_inherit_environment_and_apply_proxy_overlays() {
    if std::env::var(CHILD).as_deref() != Ok("1") {
        // Isolate the parent environment without racing other tests' reads.
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "environment::shell_processes_inherit_environment_and_apply_proxy_overlays",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(CUSTOM, "inherited-test-value")
            .env_remove("BASH_ENV")
            .env_remove("ENV")
            .env("NO_COLOR", "0");
        for name in proxy_names() {
            command.env(name, INHERITED);
        }
        for name in bypass_names() {
            command.env(name, "inherited.invalid");
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            command.env(
                "HANAFUBUKI_EXEC_NON_UTF8",
                std::ffi::OsString::from_vec(vec![0xff]),
            );
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    for tty in [false, true] {
        for (mode, expected) in [
            ("system", INHERITED),
            ("request", REQUEST),
            ("explicit", EXPLICIT),
            ("disabled", ""),
        ] {
            let environment = ChildProcessEnvironment::default();
            if matches!(mode, "explicit" | "disabled") {
                let value = (mode == "explicit").then(|| EXPLICIT.to_string());
                let overlay: HashMap<_, _> = proxy_names()
                    .iter()
                    .map(|name| (name.to_string(), value.clone()))
                    .chain(bypass_names().iter().map(|name| {
                        (
                            name.to_string(),
                            Some("localhost,127.0.0.1,::1".to_string()),
                        )
                    }))
                    .collect();
                environment.replace(overlay);
            }
            let overrides = if mode == "system" {
                BTreeMap::new()
            } else {
                proxy_names()
                    .iter()
                    .map(|name| (name.to_string(), REQUEST.to_string()))
                    .collect()
            };
            let output = run_shell(environment, overrides, tty);
            let lines: Vec<_> = output.lines().map(str::trim).collect();
            for name in proxy_names() {
                assert!(
                    lines.contains(&format!("{name}={expected}").as_str()),
                    "{mode}, tty={tty}: {output}"
                );
            }
            let bypass = if matches!(mode, "explicit" | "disabled") {
                "localhost,127.0.0.1,::1"
            } else {
                "inherited.invalid"
            };
            for name in bypass_names() {
                assert!(
                    lines.contains(&format!("{name}={bypass}").as_str()),
                    "{mode}, tty={tty}: {output}"
                );
            }
            assert!(
                lines.contains(&format!("{CUSTOM}=inherited-test-value").as_str()),
                "{output}"
            );
            assert!(lines.contains(&"NO_COLOR=1"), "{output}");
        }
    }
}

fn proxy_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"]
    } else {
        &[
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]
    }
}

fn bypass_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["NO_PROXY"]
    } else {
        &["NO_PROXY", "no_proxy"]
    }
}

fn run_shell(
    environment: ChildProcessEnvironment,
    overrides: BTreeMap<String, String>,
    tty: bool,
) -> String {
    let service = AgentExecutionService::new(ExecutorManager::new(environment));
    let names = proxy_names()
        .iter()
        .chain(bypass_names())
        .copied()
        .chain([CUSTOM, "NO_COLOR"]);
    let script = names.map(|name| {
        if cfg!(windows) {
            format!("[Console]::WriteLine('{name}=' + [Environment]::GetEnvironmentVariable('{name}'))")
        } else {
            format!("printf '%s\\n' \"{name}=${{{name}-}}\"")
        }
    }).collect::<Vec<_>>().join("; ");
    let request = ExecCommandRequest {
        cmd: script,
        shell: Some(if cfg!(windows) {
            PlatformShell::PowerShell
        } else {
            PlatformShell::Posix
        }),
        login: Some(false),
        tty,
        yield_time_ms: 1_000,
        max_output_bytes: Some(4096),
        timeout_ms: 30_000,
        workdir: Some(std::env::current_dir().unwrap()),
        shell_environment: Default::default(),
        environment: overrides,
        origin: ApprovalOrigin::Model,
        sandbox: SandboxSettings::full_access(),
        sandbox_escalation: None,
    };
    let action = service.prepare_exec_command_action(&request).unwrap();
    let scope = AgentExecutionScope::for_agent_session(83).unwrap();
    let cancellation = CancellationToken::default();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &cancellation,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut output = String::new();
    loop {
        assert!(Instant::now() < deadline, "the shell did not exit");
        let result = service
            .wait_unified_execution(
                &scope,
                started.execution_id,
                1_000,
                Some(4096),
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
