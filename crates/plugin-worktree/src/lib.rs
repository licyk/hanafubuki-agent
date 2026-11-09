//! Git worktrees for the Agent: provides `hanafubuki-agent-worktree` as a
//! [`WorktreeService`] under the plugin's name, which the facade's worktree
//! calls resolve per call; with the plugin off or failed, only worktrees are
//! unavailable.

mod plugin;

pub use plugin::{worktree_plugin, WorktreeService, WORKTREE_PLUGIN};
