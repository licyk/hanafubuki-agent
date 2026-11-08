//! Generic argv command preparation.
//!
//! No production tool starts a raw argv command; this is the harness the
//! contract tests use to drive the shared execution lifecycle, so it is compiled
//! only for tests and the `test-support` feature.

use std::collections::BTreeMap;
use std::path::Path;

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, PermissionActionDetails,
};
use hanafubuki_agent_process::executor::ExecutorIoMode;
use serde::Serialize;

use crate::environment::build_clean_command_environment;
use crate::model::{AgentExecutionError, ResolvedCommandPlan, StartCommandRequest};
use crate::resolver::{
    canonical_directory, resolve_executable, utf8_path, validate_execution_origin,
    DEFAULT_COMMAND_OUTPUT_BYTES, DEFAULT_COMMAND_TIMEOUT_MS,
};

const MAX_COMMAND_ARGUMENTS: usize = 128;
const MAX_COMMAND_ARGUMENT_BYTES: usize = 16 * 1024;
const MAX_COMMAND_TOTAL_BYTES: usize = 64 * 1024;
const MAX_COMMAND_ENVIRONMENT_ENTRIES: usize = 64;
const MAX_COMMAND_ENVIRONMENT_BYTES: usize = 64 * 1024;
const MAX_COMMAND_TIMEOUT_MS: u64 = 30 * 60 * 1_000;
const MAX_COMMAND_OUTPUT_BYTES: usize = 1024 * 1024;

pub struct PreparedCommand {
    plan: ResolvedCommandPlan,
    action: ApprovalAction,
}

impl PreparedCommand {
    pub fn action(&self) -> &ApprovalAction {
        &self.action
    }

    pub(crate) fn into_plan(self) -> ResolvedCommandPlan {
        self.plan
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalCommandAction<'a> {
    version: &'static str,
    program: &'a str,
    argv: &'a [String],
    cwd: &'a str,
    environment: &'a BTreeMap<String, String>,
    timeout_ms: u64,
    output_bytes_cap: usize,
    stdin_mode: &'static str,
    origin: &'a ApprovalOrigin,
}

pub fn prepare_command(
    request: &StartCommandRequest,
) -> Result<PreparedCommand, AgentExecutionError> {
    validate_request(request)?;
    let cwd = canonical_directory(request.cwd.as_deref().unwrap_or_else(|| Path::new(".")))?;
    let environment = build_clean_command_environment(&request.environment)?;
    let program = resolve_executable(&request.argv[0], &cwd, &environment)?;
    let program_text = utf8_path(&program, "resolved executable")?;
    let cwd_text = utf8_path(&cwd, "resolved working directory")?;
    let mut command = request.argv.clone();
    command[0] = program_text.to_string();
    let timeout_ms = request.timeout_ms.unwrap_or(DEFAULT_COMMAND_TIMEOUT_MS);
    let output_bytes_cap = request
        .output_bytes_cap
        .unwrap_or(DEFAULT_COMMAND_OUTPUT_BYTES);
    validate_execution_origin(&request.origin, None)?;
    let canonical = serde_json::to_vec(&CanonicalCommandAction {
        version: "agent-command-v3",
        program: program_text,
        argv: &command,
        cwd: cwd_text,
        environment: &environment,
        timeout_ms,
        output_bytes_cap,
        stdin_mode: "closed",
        origin: &request.origin,
    })
    .map_err(|error| AgentExecutionError::Resolution(error.to_string()))?;
    let summary = command_summary(&command, cwd_text, environment.keys());
    let action = ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::CommandExecution,
        ApprovalRiskClass::Critical,
        request.origin.clone(),
        summary,
        &canonical,
    )
    .and_then(|action| {
        action.with_details(PermissionActionDetails::Command {
            program: program_text.to_string(),
            argv: command.clone(),
            cwd: cwd_text.to_string(),
        })
    })
    .map_err(|error| AgentExecutionError::Resolution(error.to_string()))?;
    Ok(PreparedCommand {
        plan: ResolvedCommandPlan {
            command,
            cwd,
            environment,
            timeout_ms,
            output_bytes_cap,
            io_mode: ExecutorIoMode::Pipe,
            cols: None,
            rows: None,
            interactive: false,
        },
        action,
    })
}

fn validate_request(request: &StartCommandRequest) -> Result<(), AgentExecutionError> {
    if request.argv.is_empty() || request.argv[0].trim().is_empty() {
        return Err(AgentExecutionError::InvalidRequest(
            "argv must contain a non-empty executable".to_string(),
        ));
    }
    if request.argv.len() > MAX_COMMAND_ARGUMENTS {
        return Err(AgentExecutionError::InvalidRequest(format!(
            "argv exceeds {MAX_COMMAND_ARGUMENTS} entries"
        )));
    }
    let mut total = 0usize;
    for argument in &request.argv {
        if argument.as_bytes().contains(&0) || argument.len() > MAX_COMMAND_ARGUMENT_BYTES {
            return Err(AgentExecutionError::InvalidRequest(
                "argv contains an invalid or oversized argument".to_string(),
            ));
        }
        total = total.saturating_add(argument.len());
    }
    if total > MAX_COMMAND_TOTAL_BYTES {
        return Err(AgentExecutionError::InvalidRequest(format!(
            "argv exceeds {MAX_COMMAND_TOTAL_BYTES} bytes"
        )));
    }
    if request.environment.len() > MAX_COMMAND_ENVIRONMENT_ENTRIES {
        return Err(AgentExecutionError::InvalidRequest(format!(
            "environment exceeds {MAX_COMMAND_ENVIRONMENT_ENTRIES} entries"
        )));
    }
    let environment_bytes = request
        .environment
        .iter()
        .fold(0usize, |size, (key, value)| {
            size.saturating_add(key.len()).saturating_add(value.len())
        });
    if environment_bytes > MAX_COMMAND_ENVIRONMENT_BYTES {
        return Err(AgentExecutionError::InvalidRequest(format!(
            "environment exceeds {MAX_COMMAND_ENVIRONMENT_BYTES} bytes"
        )));
    }
    if request
        .timeout_ms
        .is_some_and(|timeout| timeout == 0 || timeout > MAX_COMMAND_TIMEOUT_MS)
    {
        return Err(AgentExecutionError::InvalidRequest(format!(
            "timeout must be between 1 and {MAX_COMMAND_TIMEOUT_MS} ms"
        )));
    }
    if request
        .output_bytes_cap
        .is_some_and(|cap| cap == 0 || cap > MAX_COMMAND_OUTPUT_BYTES)
    {
        return Err(AgentExecutionError::InvalidRequest(format!(
            "output cap must be between 1 and {MAX_COMMAND_OUTPUT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn command_summary<'a>(
    command: &[String],
    cwd: &str,
    environment_keys: impl Iterator<Item = &'a String>,
) -> String {
    let mut summary = format!("Run argv {:?} in {cwd}", command);
    let keys = environment_keys.cloned().collect::<Vec<_>>();
    if !keys.is_empty() {
        summary.push_str(&format!(" with environment keys {keys:?}"));
    }
    const MAX_SUMMARY: usize = 1_900;
    if summary.len() > MAX_SUMMARY {
        let boundary = (0..=MAX_SUMMARY)
            .rev()
            .find(|index| summary.is_char_boundary(*index))
            .unwrap_or(0);
        summary.truncate(boundary);
        summary.push('…');
    }
    summary
}
