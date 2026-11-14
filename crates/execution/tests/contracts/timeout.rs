//! A command's timeout is its request's: its plan starts the process with
//! it, its approval is of it, and the executor stops the command past it.

use super::*;

use std::time::Instant;

use hanafubuki_agent_execution::ExecCommandRequest;
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_sandbox::SandboxSettings;

fn request(cmd: &str, timeout_ms: u64) -> ExecCommandRequest {
    ExecCommandRequest {
        cmd: cmd.to_string(),
        shell: None,
        login: false,
        tty: false,
        yield_time_ms: 10,
        max_output_bytes: None,
        timeout_ms,
        workdir: Some(std::env::current_dir().unwrap()),
        environment: BTreeMap::new(),
        origin: ApprovalOrigin::Model,
        sandbox: SandboxSettings::full_access(),
        sandbox_escalation: None,
    }
}

#[test]
fn the_plan_and_the_approval_carry_the_requests_timeout() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(71).unwrap();
    let ninety_minutes = request("serve", 90 * 60_000);
    let action = service
        .prepare_exec_command_action(&ninety_minutes)
        .unwrap();
    let pty = ExecCommandRequest {
        tty: true,
        ..request("serve", 2 * 60_000)
    };
    for (request, action) in [
        (ninety_minutes.clone(), action.clone()),
        (
            pty.clone(),
            service.prepare_exec_command_action(&pty).unwrap(),
        ),
    ] {
        service
            .start_exec_command(
                scope.clone(),
                request,
                Some(&approve_action(&action)),
                &CancellationToken::default(),
            )
            .unwrap();
    }
    let starts = process.starts();
    assert_eq!(starts[0].timeout_ms, Some(90 * 60_000));
    assert_eq!(
        (starts[1].io_mode, starts[1].timeout_ms),
        (ExecutorIoMode::Pty, Some(2 * 60_000))
    );

    let digest =
        |request: &ExecCommandRequest| service.prepare_exec_command_action(request).unwrap().digest;
    assert_eq!(digest(&ninety_minutes), action.digest);
    assert_ne!(
        digest(&request("serve", 10 * 60_000)),
        action.digest,
        "an approval of one timeout is not of another"
    );
    let refused = service.prepare_exec_command_action(&request("serve", 0));
    assert!(
        matches!(refused, Err(AgentExecutionError::InvalidRequest(_))),
        "the executor would take 0 as no limit: {refused:?}"
    );
}

/// The executor's own timer, with a timeout of a second (the option counts
/// minutes; the request, milliseconds).
#[test]
fn a_command_past_its_timeout_is_stopped_timed_out() {
    #[cfg(unix)]
    let sleep = "sleep 60";
    #[cfg(windows)]
    let sleep = "Start-Sleep -Seconds 60";
    let service = AgentExecutionService::new(ExecutorManager::default());
    let scope = AgentExecutionScope::for_agent_session(72).unwrap();
    let request = request(sleep, 1_000);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started_at = Instant::now();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    let outcome = loop {
        let waited = service
            .wait(
                &scope,
                started.execution_id,
                30_000,
                &CancellationToken::default(),
            )
            .unwrap();
        if waited.completed {
            break waited;
        }
        assert!(
            started_at.elapsed() < Duration::from_secs(50),
            "still running"
        );
    };
    assert_eq!(outcome.snapshot.status, AgentExecutionStatus::TimedOut);
    service.dispose(&scope, started.execution_id).unwrap();
}
