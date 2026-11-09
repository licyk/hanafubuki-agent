use std::path::{Path, PathBuf};

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, PermissionActionDetails,
};
use hanafubuki_agent_process::executor::ExecutorIoMode;
use hanafubuki_agent_process::terminal_size::TerminalSize;
use serde::Serialize;

use crate::environment::build_clean_command_environment;
use crate::model::{AgentExecutionError, ExecCommandRequest, PlatformShell, ResolvedCommandPlan};
use crate::resolver::{
    canonical_directory, resolve_executable, utf8_path, validate_execution_origin,
    DEFAULT_COMMAND_OUTPUT_BYTES, DEFAULT_COMMAND_TIMEOUT_MS, MAX_WAIT_MS,
};
use sha2::{Digest, Sha256};

const MAX_SHELL_SCRIPT_BYTES: usize = 48 * 1024;
/// Prefixed to every PowerShell script so its console output is UTF-8 rather
/// than the system code page (Codex parity).
pub const POWERSHELL_UTF8_OUTPUT_PREFIX: &str =
    "try { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8 } catch {}\n";

pub(crate) struct PreparedExecCommand {
    pub(crate) plan: ResolvedCommandPlan,
    action: ApprovalAction,
}

impl PreparedExecCommand {
    pub(crate) fn action(&self) -> &ApprovalAction {
        &self.action
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalShellAction<'a> {
    version: &'static str,
    shell: PlatformShell,
    program: &'a str,
    argv: &'a [String],
    script: &'a str,
    cwd: &'a str,
    environment: &'a std::collections::BTreeMap<String, String>,
    timeout_ms: u64,
    output_bytes_cap: usize,
    io_mode: ExecutorIoMode,
    cols: Option<u16>,
    rows: Option<u16>,
    /// Outside the sandbox: an approval of the sandboxed command must not
    /// authorize it. The justification is the model's wording, not part of
    /// what runs.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    unsandboxed: bool,
}

/// Prepares `request` to run in its `workdir`, which resolves against
/// `working_directory`, never the process's own: that is wherever the Agent
/// was started from, and `-C` does not change it.
pub(crate) fn prepare_exec_command(
    request: &ExecCommandRequest,
    working_directory: Option<&Path>,
) -> Result<PreparedExecCommand, AgentExecutionError> {
    if request.cmd.is_empty()
        || request.cmd.len() > MAX_SHELL_SCRIPT_BYTES
        || request.cmd.as_bytes().contains(&0)
        || request.yield_time_ms == 0
        || request.yield_time_ms > MAX_WAIT_MS
    {
        return Err(AgentExecutionError::InvalidRequest(format!(
            "command must contain 1 to {MAX_SHELL_SCRIPT_BYTES} bytes without NUL and yield_time must be between 1 and {MAX_WAIT_MS} ms"
        )));
    }
    let shell = request.shell.unwrap_or_else(default_platform_shell);
    let source_digest = hex::encode(Sha256::digest(request.cmd.as_bytes()));
    validate_execution_origin(&request.origin, Some(&source_digest))?;
    let cwd = canonical_directory(&command_directory(
        request.workdir.as_deref(),
        working_directory,
    )?)?;
    let environment = build_clean_command_environment(&request.environment)?;
    let program = resolve_shell(shell, &cwd, &environment)?;
    let (io_mode, cols, rows) = if request.tty {
        (
            ExecutorIoMode::Pty,
            Some(TerminalSize::DEFAULT.cols),
            Some(TerminalSize::DEFAULT.rows),
        )
    } else {
        (ExecutorIoMode::Pipe, None, None)
    };
    let plan = ResolvedCommandPlan {
        command: exec_shell_command(shell, program, &request.cmd, request.login),
        cwd,
        environment,
        timeout_ms: DEFAULT_COMMAND_TIMEOUT_MS,
        output_bytes_cap: request
            .max_output_bytes
            .unwrap_or(DEFAULT_COMMAND_OUTPUT_BYTES),
        io_mode,
        cols,
        rows,
        interactive: request.tty,
    };
    let summary = match &request.sandbox_escalation {
        None => format!(
            "Run {:?} command in {}: {}",
            shell,
            plan.cwd.display(),
            bounded_summary(&request.cmd)
        ),
        Some(escalation) => format!(
            "Run {:?} command outside the sandbox in {}: {}{}",
            shell,
            plan.cwd.display(),
            bounded_summary(&request.cmd),
            escalation
                .justification
                .as_deref()
                .map(|justification| format!(" (reason: {})", bounded_summary(justification)))
                .unwrap_or_default()
        ),
    };
    let mut action = shell_action(
        shell,
        &plan,
        &request.cmd,
        summary,
        &request.origin,
        request.sandbox_escalation.is_some(),
    )?;
    if let Some(escalation) = &request.sandbox_escalation {
        action = action.with_sandbox_escalation(escalation.clone());
    }
    Ok(PreparedExecCommand { plan, action })
}

fn command_directory(
    workdir: Option<&Path>,
    working_directory: Option<&Path>,
) -> Result<PathBuf, AgentExecutionError> {
    match (workdir, working_directory) {
        (Some(workdir), _) if workdir.is_absolute() => Ok(workdir.to_path_buf()),
        (Some(workdir), Some(base)) => Ok(base.join(workdir)),
        (None, Some(base)) => Ok(base.to_path_buf()),
        (_, None) => Err(AgentExecutionError::Resolution(
            "the command has no absolute workdir and the Agent has no working directory"
                .to_string(),
        )),
    }
}

const fn default_platform_shell() -> PlatformShell {
    #[cfg(windows)]
    {
        PlatformShell::PowerShell
    }
    #[cfg(not(windows))]
    {
        PlatformShell::Posix
    }
}

fn resolve_shell(
    shell: PlatformShell,
    cwd: &Path,
    environment: &std::collections::BTreeMap<String, String>,
) -> Result<PathBuf, AgentExecutionError> {
    match shell {
        PlatformShell::Posix => {
            #[cfg(unix)]
            {
                for candidate in ["/bin/bash", "/bin/sh"] {
                    if let Ok(program) = resolve_executable(candidate, cwd, environment) {
                        return Ok(program);
                    }
                }
                Err(AgentExecutionError::Resolution(
                    "no supported POSIX shell was found".to_string(),
                ))
            }
            #[cfg(not(unix))]
            {
                Err(AgentExecutionError::Resolution(
                    "POSIX shell is unavailable on this platform".to_string(),
                ))
            }
        }
        PlatformShell::PowerShell => {
            for candidate in ["pwsh", "powershell"] {
                if let Ok(program) = resolve_executable(candidate, cwd, environment) {
                    return Ok(program);
                }
            }
            Err(AgentExecutionError::Resolution(
                "PowerShell was not found in the clean PATH".to_string(),
            ))
        }
        PlatformShell::Cmd => {
            #[cfg(windows)]
            {
                resolve_executable("cmd.exe", cwd, environment)
            }
            #[cfg(not(windows))]
            {
                Err(AgentExecutionError::Resolution(
                    "cmd is unavailable on this platform".to_string(),
                ))
            }
        }
    }
}

fn exec_shell_command(
    shell: PlatformShell,
    program: PathBuf,
    command_text: &str,
    login: bool,
) -> Vec<String> {
    let mut command = vec![program.to_string_lossy().into_owned()];
    match shell {
        PlatformShell::Posix => {
            command.push(if login { "-lc" } else { "-c" }.to_string());
            command.push(command_text.to_string());
        }
        PlatformShell::PowerShell => command.extend([
            "-NoLogo".to_string(),
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            format!("{POWERSHELL_UTF8_OUTPUT_PREFIX}{command_text}"),
        ]),
        PlatformShell::Cmd => command.extend([
            "/D".to_string(),
            "/S".to_string(),
            "/C".to_string(),
            command_text.to_string(),
        ]),
    }
    command
}

fn shell_action(
    shell: PlatformShell,
    plan: &ResolvedCommandPlan,
    script: &str,
    summary: String,
    origin: &ApprovalOrigin,
    unsandboxed: bool,
) -> Result<ApprovalAction, AgentExecutionError> {
    let canonical = serde_json::to_vec(&CanonicalShellAction {
        version: "agent-shell-v4",
        shell,
        program: &plan.command[0],
        argv: &plan.command,
        script,
        cwd: utf8_path(&plan.cwd, "resolved working directory")?,
        environment: &plan.environment,
        timeout_ms: plan.timeout_ms,
        output_bytes_cap: plan.output_bytes_cap,
        io_mode: plan.io_mode,
        cols: plan.cols,
        rows: plan.rows,
        unsandboxed,
    })
    .map_err(|error| AgentExecutionError::Resolution(error.to_string()))?;
    ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::ShellExecution,
        ApprovalRiskClass::Critical,
        origin.clone(),
        bounded_action_summary(&summary),
        &canonical,
    )
    .and_then(|action| {
        action.with_details(PermissionActionDetails::Command {
            program: plan.command[0].clone(),
            argv: plan.command.clone(),
            cwd: plan.cwd.to_string_lossy().into_owned(),
        })
    })
    .map_err(|error| AgentExecutionError::Resolution(error.to_string()))
}

fn bounded_summary(value: &str) -> String {
    const LIMIT: usize = 1_500;
    if value.len() <= LIMIT {
        return value.to_string();
    }
    let boundary = (0..=LIMIT)
        .rev()
        .find(|index| value.is_char_boundary(*index))
        .unwrap_or(0);
    format!("{}…", &value[..boundary])
}

fn bounded_action_summary(value: &str) -> String {
    const LIMIT: usize = 2_000;
    if value.len() <= LIMIT {
        return value.to_string();
    }
    let boundary = (0..=LIMIT)
        .rev()
        .find(|index| value.is_char_boundary(*index))
        .unwrap_or(0);
    format!("{}…", &value[..boundary])
}
