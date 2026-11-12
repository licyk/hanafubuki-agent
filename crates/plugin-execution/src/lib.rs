//! Command execution for the Agent: binds `hanafubuki-agent-execution` to the tool
//! registry as `exec_command`, `write_stdin` and `resize_execution`, and
//! streams each execution's output into the turn's activity monitor, and
//! presents their calls (`presenter`). A
//! `git clone` it runs always needs a person's approval.

mod approval;
mod arguments;
mod monitor;
mod plugin;
mod presenter;
mod specs;
mod tool;

pub use plugin::execution_plugin;
