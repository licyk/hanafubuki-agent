//! The sandbox of the commands the model runs, ported from Codex's
//! `sandboxing` and `linux-sandbox`: a [`SandboxPolicy`] (read-only,
//! workspace-write or full access) and the [`Sandbox`] that enforces it by
//! rewriting a command's argv before it is spawned.
//!
//! On Linux the rewritten command re-executes the Agent's own binary as a
//! helper ([`helper_dispatch`]), which confines itself with Landlock
//! (filesystem) and seccomp (network) and then executes the command in
//! place, so the process the `ExecutorManager` owns is the command itself.
//! Other platforms have no backend yet: a policy other than full access
//! fails there rather than run unconfined.

mod backend;
mod helper;
#[cfg(target_os = "linux")]
mod linux;
mod policy;

pub use backend::{Sandbox, SandboxError, SandboxStatus};
pub use hanafubuki_agent_foundation::agent_settings::AgentSandboxMode;
pub use helper::{helper_dispatch, HELPER_ARG, HELPER_FAILURE_EXIT_CODE};
pub use policy::{platform_default_mode, SandboxPolicy, SandboxSettings};
