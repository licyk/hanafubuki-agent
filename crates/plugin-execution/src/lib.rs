//! Command execution for the Agent: binds `hanafubuki-agent-execution` to the tool
//! registry as `exec_command`, `write_stdin` and `resize_execution`, and
//! streams each execution's output into the turn's activity monitor, and
//! presents their calls (`presenter`). A
//! `git clone` it runs always needs a person's approval. Its option
//! `commandTimeoutMinutes` bounds each command from its start.

mod approval;
mod arguments;
mod monitor;
mod options;
mod plugin;
mod presenter;
mod specs;
mod tool;

pub use options::{COMMAND_TIMEOUT_MINUTES_LIMITS, DEFAULT_COMMAND_TIMEOUT_MINUTES};
pub use plugin::execution_plugin;
