//! A POSIX shell's command in the Windows sandbox, as `exec_command` runs
//! it. This binary answers the sandbox helper's command line first, as the
//! Agent's does, so the commands it confines start it again as the helper;
//! `main` runs the check itself, without a test harness.

fn main() {
    hanafubuki_agent_sandbox::helper_dispatch();
    #[cfg(windows)]
    windows::run();
}

#[cfg(windows)]
mod windows {
    use std::collections::BTreeMap;
    use std::io::Write as _;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use hanafubuki_agent_approval::test_support::ManualApprovalClock;
    use hanafubuki_agent_approval::{
        ApprovalContext, ApprovalDecisionSource, ApprovalOrigin, ApprovalService,
    };
    use hanafubuki_agent_execution::{
        AgentExecutionId, AgentExecutionScope, AgentExecutionService, ExecCommandRequest,
        PlatformShell, WriteStdinRequest,
    };
    use hanafubuki_agent_foundation::cancellation::CancellationToken;
    use hanafubuki_agent_process::executor::ExecutorManager;
    use hanafubuki_agent_sandbox::{AgentSandboxMode, AgentWindowsSandbox, SandboxSettings};

    const DEADLINE: Duration = Duration::from_secs(240);

    /// Run only when asked for (`-- --ignored`), as libtest runs an
    /// ignored test.
    const IGNORED: &str = "the bash of Git for Windows and of MSYS2 stops at its start under \
        the sandbox's restricted token: `couldn't create signal pipe, Win32 error 5`";

    pub(super) fn run() {
        if !std::env::args().any(|argument| argument == "--ignored") {
            report(&format!(
                "a_posix_command_writes_only_the_workspace ignored: {IGNORED}"
            ));
            return;
        }
        let root = std::env::var("SystemRoot").unwrap();
        let nowhere = std::env::temp_dir()
            .join("hanafubuki-agent-no-git")
            .display()
            .to_string();
        let msys2 = BTreeMap::from([
            ("PATH".to_string(), format!(r"{root}\System32;{root}")),
            ("ProgramFiles".to_string(), nowhere.clone()),
            ("ProgramFiles(x86)".to_string(), nowhere.clone()),
            ("LOCALAPPDATA".to_string(), nowhere),
        ]);
        let mut failures = Vec::new();
        for environment in [BTreeMap::new(), msys2] {
            for tty in [false, true] {
                if let Err(failure) =
                    a_posix_command_writes_only_the_workspace(environment.clone(), tty)
                {
                    report(&failure);
                    failures.push(failure);
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The command runs whole (its quoting, the MSYS programs) and writes
    /// in the workspace, but not outside it.
    fn a_posix_command_writes_only_the_workspace(
        environment: BTreeMap<String, String>,
        tty: bool,
    ) -> Result<(), String> {
        let workspace = directory();
        let outside = directory();
        let escaped = outside.path().join("file");
        let service = AgentExecutionService::new(ExecutorManager::default());
        service.set_workspace_roots(vec![workspace.path().to_path_buf()]);
        let request = ExecCommandRequest {
            cmd: format!(
                "printf '<%s>\\n' \"a\\\"b\" '*' 'c\\\\d'; ls >/dev/null && echo ok=ls; \
                 printf written > inside; \
                 if printf x > '{}' 2>/dev/null; then echo ok=outside; else echo refused=outside; fi",
                escaped.display()
            ),
            shell: Some(PlatformShell::Posix),
            login: false,
            tty,
            yield_time_ms: 30_000,
            max_output_bytes: None,
            timeout_ms: 600_000,
            workdir: None,
            environment,
            origin: ApprovalOrigin::Model,
            sandbox: SandboxSettings {
                mode: AgentSandboxMode::WorkspaceWrite,
                writable_roots: Vec::new(),
                network_access: false,
                windows_sandbox: Some(AgentWindowsSandbox::Unelevated),
            },
            sandbox_escalation: None,
        };
        let action = match service.prepare_exec_command_action(&request) {
            Ok(action) => action,
            Err(error) => {
                report(&format!("skipped, no POSIX shell: {error}"));
                return Ok(());
            }
        };
        let scope = AgentExecutionScope::for_agent_session(71).unwrap();
        let started = service
            .start_exec_command(
                scope.clone(),
                request,
                Some(&approve(&action)),
                &CancellationToken::default(),
            )
            .unwrap();
        let began = Instant::now();
        let mut result = service
            .wait_unified_execution(
                &scope,
                started.execution_id,
                30_000,
                None,
                &CancellationToken::default(),
            )
            .unwrap();
        let mut output = std::mem::take(&mut result.output);
        while let Some(id) = result.session_id {
            assert!(began.elapsed() < DEADLINE, "still running: {output:?}");
            result = service
                .write_stdin(
                    &scope,
                    WriteStdinRequest {
                        session_id: AgentExecutionId::from_untrusted(id).unwrap(),
                        chars: String::new(),
                        yield_time_ms: 30_000,
                        max_output_bytes: None,
                    },
                    &CancellationToken::default(),
                )
                .unwrap();
            output.push_str(&result.output);
        }
        let lines = output.lines().map(str::trim_end).collect::<Vec<_>>();
        let failed = |what: &str| {
            Err(format!(
                "{} tty={tty}: {what}: {output:?} (exit {:?})",
                action.summary, result.exit_code
            ))
        };
        for expected in ["<a\"b>", "<*>", "<c\\\\d>", "ok=ls", "refused=outside"] {
            if !lines.contains(&expected) {
                return failed(&format!("no line {expected:?}"));
            }
        }
        if std::fs::read_to_string(workspace.path().join("inside"))
            .ok()
            .as_deref()
            != Some("written")
        {
            return failed("nothing written in the workspace");
        }
        if escaped.exists() {
            return failed("written outside the workspace");
        }
        report(&format!("ok: tty={tty}: {}", action.summary));
        Ok(())
    }

    /// Directories outside `TEMP`, which workspace-write always allows.
    fn directory() -> tempfile::TempDir {
        tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
    }

    fn approve(
        action: &hanafubuki_agent_approval::ApprovalAction,
    ) -> hanafubuki_agent_approval::AuthorizedAction {
        let context =
            ApprovalContext::from_canonical_bytes("call-1".to_string(), b"scope-1").unwrap();
        ApprovalService::new(Arc::new(ManualApprovalClock::new(1)))
            .authorize(
                action.clone(),
                &context,
                Duration::from_secs(1),
                ApprovalDecisionSource::FullAccess,
            )
            .unwrap()
    }

    fn report(line: &str) {
        let _ = writeln!(std::io::stderr(), "windows_sandbox: {line}");
    }
}
