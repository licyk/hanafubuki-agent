use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_approval::test_support::ManualApprovalClock;
use hanafubuki_agent_approval::{
    ApprovalContext, ApprovalDecisionSource, ApprovalOrigin, ApprovalService,
};
use hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort;
use hanafubuki_agent_execution::{
    prepare_command, AgentExecutionError, AgentExecutionScope, AgentExecutionService,
    AgentExecutionStatus, StartCommandRequest, UnifiedExecResult,
};
#[cfg(unix)]
use hanafubuki_agent_execution::{
    AgentExecutionGeneration, AgentExecutionKind, ExecCommandRequest, WriteStdinRequest,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_process::executor::{EnvironmentInheritance, ExecutorIoMode};
#[cfg(unix)]
use hanafubuki_agent_sandbox::SandboxSettings;

fn command_request() -> StartCommandRequest {
    StartCommandRequest {
        argv: vec![test_program(), "hello".to_string()],
        cwd: Some(std::env::current_dir().unwrap()),
        environment: BTreeMap::from([("HANAFUBUKI_AGENT_TEST".to_string(), "present".to_string())]),
        timeout_ms: Some(1_000),
        output_bytes_cap: Some(32),
        origin: ApprovalOrigin::Model,
    }
}

#[cfg(unix)]
fn test_program() -> String {
    "/bin/echo".to_string()
}

#[cfg(windows)]
fn test_program() -> String {
    std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn approved(request: &StartCommandRequest) -> hanafubuki_agent_approval::AuthorizedAction {
    let prepared = prepare_command(request).unwrap();
    approve_action(prepared.action())
}

fn approve_action(
    action: &hanafubuki_agent_approval::ApprovalAction,
) -> hanafubuki_agent_approval::AuthorizedAction {
    let context = ApprovalContext::from_canonical_bytes("call-1".to_string(), b"scope-1").unwrap();
    ApprovalService::new(Arc::new(ManualApprovalClock::new(1)))
        .authorize(
            action.clone(),
            &context,
            Duration::from_secs(1),
            ApprovalDecisionSource::FullAccess,
        )
        .unwrap()
}

#[cfg(unix)]
fn exec_request(tty: bool) -> ExecCommandRequest {
    ExecCommandRequest {
        cmd: "printf unified".to_string(),
        shell: None,
        login: true,
        tty,
        yield_time_ms: 10,
        max_output_bytes: Some(4096),
        workdir: Some(std::env::current_dir().unwrap()),
        environment: BTreeMap::new(),
        origin: ApprovalOrigin::Model,
        sandbox: SandboxSettings::full_access(),
        sandbox_escalation: None,
    }
}

mod cleanup;
#[cfg(unix)]
mod output;
#[cfg(unix)]
mod ownership;
mod platform;
mod unified;
#[cfg(windows)]
mod windows_posix;
#[cfg(windows)]
mod windows_powershell;
