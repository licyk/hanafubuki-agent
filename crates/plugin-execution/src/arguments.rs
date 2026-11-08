//! The model's execution tool arguments, validated into domain requests.

use std::path::PathBuf;

use hanafubuki_agent_approval::ApprovalOrigin;
use hanafubuki_agent_contracts::tools::arguments::{
    clamped_u64, nullable_bool, nullable_bool_or_default, nullable_bounded_string,
    nullable_text_or_default, optional_clamped_usize, required_bounded_string, required_u64,
    ArgumentNotes,
};
use hanafubuki_agent_contracts::{object_arguments, ToolCallValidationError};
use hanafubuki_agent_execution::{ExecCommandRequest, PlatformShell};
use serde_json::Value;

use crate::tool::ExecToolKind;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExecCall {
    ExecCommand(ExecCommandRequest),
    WriteStdin {
        session_id: u64,
        chars: String,
        yield_time_ms: u64,
        max_output_bytes: Option<usize>,
    },
    Resize {
        session_id: u64,
        cols: u16,
        rows: u16,
    },
}

/// The bounds the schemas in `specs.rs` state. A wait or an output cap out of
/// range is clamped into them (never past them) with a note; a terminal size
/// likewise.
const MAX_YIELD_TIME_MS: u64 = 30_000;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_TERMINAL_CELLS: u64 = 500;

/// Parses a call and the notes on how its limits were adjusted. An omitted
/// nullable field is absent, like `null`; an unknown key is ignored, since
/// the executor's capabilities (argv, receipts, confirmations) are never
/// arguments, so no key the model adds can reach one.
pub(crate) fn parse(
    kind: ExecToolKind,
    raw: &Value,
) -> Result<(ExecCall, ArgumentNotes), ToolCallValidationError> {
    let arguments = object_arguments(raw)?;
    let mut notes = ArgumentNotes::default();
    let call = match kind {
        ExecToolKind::ExecCommand => {
            let shell = nullable_bounded_string(arguments, "shell", 32)?
                .map(|shell| match shell.as_str() {
                    "posix" => Ok(PlatformShell::Posix),
                    "power_shell" => Ok(PlatformShell::PowerShell),
                    "cmd" => Ok(PlatformShell::Cmd),
                    _ => Err(ToolCallValidationError::InvalidArgument("shell")),
                })
                .transpose()?;
            ExecCall::ExecCommand(ExecCommandRequest {
                cmd: required_bounded_string(arguments, "cmd", 48 * 1024)?,
                shell,
                login: nullable_bool(arguments, "login")?.unwrap_or(true),
                tty: nullable_bool_or_default(arguments, "tty", false)?,
                yield_time_ms: clamped_u64(
                    arguments,
                    "yield_time_ms",
                    1,
                    MAX_YIELD_TIME_MS,
                    10_000,
                    &mut notes,
                )?,
                max_output_bytes: optional_clamped_usize(
                    arguments,
                    "max_output_bytes",
                    1,
                    MAX_OUTPUT_BYTES,
                    &mut notes,
                )?,
                workdir: nullable_bounded_string(arguments, "workdir", 4096)?.map(PathBuf::from),
                environment: Default::default(),
                origin: ApprovalOrigin::Model,
            })
        }
        ExecToolKind::WriteStdin => {
            let chars = nullable_text_or_default(arguments, "chars", 16 * 1024)?;
            let default_yield_time_ms = if chars.is_empty() { 5_000 } else { 250 };
            ExecCall::WriteStdin {
                session_id: required_u64(arguments, "session_id", 1, u64::MAX)?,
                chars,
                yield_time_ms: clamped_u64(
                    arguments,
                    "yield_time_ms",
                    1,
                    MAX_YIELD_TIME_MS,
                    default_yield_time_ms,
                    &mut notes,
                )?,
                max_output_bytes: optional_clamped_usize(
                    arguments,
                    "max_output_bytes",
                    1,
                    MAX_OUTPUT_BYTES,
                    &mut notes,
                )?,
            }
        }
        ExecToolKind::ResizeExecution => ExecCall::Resize {
            session_id: required_u64(arguments, "session_id", 1, u64::MAX)?,
            cols: terminal_cells(arguments, "cols", &mut notes)?,
            rows: terminal_cells(arguments, "rows", &mut notes)?,
        },
    };
    Ok((call, notes))
}

/// A terminal dimension: required, clamped into `1..=500`.
fn terminal_cells(
    arguments: &serde_json::Map<String, Value>,
    name: &'static str,
    notes: &mut ArgumentNotes,
) -> Result<u16, ToolCallValidationError> {
    if arguments.get(name).is_none_or(Value::is_null) {
        return Err(ToolCallValidationError::MissingArgument(name));
    }
    let cells = clamped_u64(arguments, name, 1, MAX_TERMINAL_CELLS, 1, notes)?;
    u16::try_from(cells).map_err(|_| ToolCallValidationError::InvalidArgument(name))
}

#[cfg(test)]
mod tests;
