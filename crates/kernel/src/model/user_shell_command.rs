// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What the model reads of a command the person typed: Codex's
//! `<user_shell_command>` fragment (`core/src/context/user_shell_command.rs`).

/// The kind of the context fragment a `UserShellCommand` item is lowered
/// to.
pub(crate) const USER_SHELL_COMMAND_FRAGMENT: &str = "user_shell_command";

/// The fragment's text. A command that did not run to an exit has no exit
/// code; its output says why.
pub(crate) fn user_shell_command_record(
    command: &str,
    exit_code: Option<i32>,
    duration_millis: u64,
    output: &str,
) -> String {
    let exit_code = exit_code.map_or_else(|| "none".to_string(), |code| code.to_string());
    let seconds = duration_millis as f64 / 1000.0;
    format!(
        "<user_shell_command>\n<command>\n{command}\n</command>\n<result>\nExit code: {exit_code}\nDuration: {seconds:.4} seconds\nOutput:\n{output}\n</result>\n</user_shell_command>"
    )
}

#[cfg(test)]
mod tests;
