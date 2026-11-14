//! How an interface shows the execution tools' calls: the command line, the
//! session written to, and what each printed.

use hanafubuki_agent_contracts::{structured_output, AgentToolDisplay, AgentToolPresenter};
use hanafubuki_agent_execution::UnifiedExecResult;
use hanafubuki_agent_tool_contracts::ToolOutput;
use serde_json::Value;

pub(crate) struct ExecPresenter;

const EXEC_COMMAND: &str = "exec_command";
const WRITE_STDIN: &str = "write_stdin";
const RESIZE_EXECUTION: &str = "resize_execution";

impl AgentToolPresenter for ExecPresenter {
    fn tools(&self) -> &[&'static str] {
        &[EXEC_COMMAND, WRITE_STDIN, RESIZE_EXECUTION]
    }

    /// The command line; for input, the session and what was typed; for a
    /// resize, the session and its size.
    fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        match tool {
            EXEC_COMMAND => text(arguments, "cmd").map(str::to_string),
            WRITE_STDIN => {
                let session = arguments.get("session_id")?.as_u64()?;
                Some(match input(arguments) {
                    Some(chars) => format!("session {session} ← {chars}"),
                    None => format!("session {session}"),
                })
            }
            RESIZE_EXECUTION => Some(format!(
                "session {} to {}×{}",
                arguments.get("session_id")?.as_u64()?,
                arguments.get("cols")?.as_u64()?,
                arguments.get("rows")?.as_u64()?
            )),
            _ => None,
        }
    }

    /// A command's output as its result returned it. A resize shows the
    /// fallback.
    fn display(
        &self,
        tool: &str,
        arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        let command = match tool {
            EXEC_COMMAND => text(arguments, "cmd")?.to_string(),
            WRITE_STDIN => input(arguments).unwrap_or_default(),
            _ => return None,
        };
        let result: UnifiedExecResult =
            serde_json::from_value(structured_output(output)?.clone()).ok()?;
        Some(AgentToolDisplay::Command {
            command,
            cwd: text(arguments, "workdir").map(str::to_string),
            output: result.output,
            exit_code: result.exit_code,
            duration_millis: Some(result.wall_time_ms),
            truncated: result.truncated,
            // The registry adds the styles the call committed beside it.
            styled: None,
        })
    }
}

fn text<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)?
        .as_str()
        .filter(|text| !text.trim().is_empty())
}

/// What `write_stdin` typed, its control characters escaped (`\u{3}` for
/// Ctrl-C); `None` for a poll.
fn input(arguments: &Value) -> Option<String> {
    let chars = arguments.get("chars")?.as_str()?;
    (!chars.is_empty()).then(|| chars.escape_debug().to_string())
}

#[cfg(test)]
mod tests;
