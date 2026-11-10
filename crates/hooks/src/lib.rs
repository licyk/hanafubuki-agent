//! Hooks for the Agent, in Codex's format: commands the user configures in
//! `hooks.json` or the `[hooks]` table of `config.toml` (in the user's
//! configuration directory, and in a trusted project's), and the hooks of
//! the enabled installed plugins, run with an event as JSON on their stdin
//! when the Agent raises it. A hook runs only once the user trusted the
//! hash of its definition (a plugin's, with its package's content hash,
//! which the package must still have when the hook runs), and while it is
//! enabled; that state is the Agent's own file. Commands run through the Agent's
//! `ExecutorManager`, bounded by their timeout.
//!
//! The plugin that binds these events to the kernel is
//! `hanafubuki-agent-plugin-hooks`.

mod command;
mod config;
mod discovery;
mod engine;
mod event;
mod matcher;
mod output;
mod plugin;
mod state;

pub use discovery::{HookCatalog, HookEntry, HookHandler, HookSource, HookSources, HookTrust};
pub use engine::{
    tool_matcher_inputs, HookEvent, HookOutcome, HookScope, HooksEngine, SESSION_START_SOURCE,
};
pub use event::HookEventName;
pub use plugin::{PluginHookDeclaration, PluginHookSource};
