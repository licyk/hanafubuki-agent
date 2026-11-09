//! The user's hooks (`hanafubuki-agent-hooks`) bound to the kernel:
//! PreToolUse and PostToolUse as a tool lifecycle hook, Stop as a
//! continuation hook, SessionStart and UserPromptSubmit as a world-state
//! section, and a [`HooksService`] under the plugin's name for the facade's
//! hook calls. With no hook configured, it adds nothing the model sees and
//! no event; with the plugin off, no hook runs.

mod context;
mod plugin;
mod runtime;
mod service;
mod stop;
mod tool_use;

pub use plugin::{hooks_plugin, WorkingDirectory, HOOKS_PLUGIN};
pub use service::HooksService;
