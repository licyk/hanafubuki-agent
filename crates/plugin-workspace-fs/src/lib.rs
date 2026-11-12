//! Workspace filesystem tools for the Agent: binds `hanafubuki-agent-workspace-fs` to the
//! tool registry as `list_directory`, `search_text`, `read_file`,
//! `apply_patch`, `move_path` and `delete_path`, and presents their calls.

mod arguments;
mod plugin;
mod presenter;
mod specs;
mod tool;

pub use plugin::workspace_fs_plugin;
