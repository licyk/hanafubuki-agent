//! The sandbox of the commands the model runs, ported from Codex's
//! `sandboxing`, `linux-sandbox` and `windows-sandbox-rs`: a
//! [`SandboxPolicy`] (read-only, workspace-write or full access) and the
//! [`Sandbox`] that enforces it by rewriting a command's argv before it is
//! spawned.
//!
//! On Linux the rewritten command re-executes the Agent's own binary as a
//! helper ([`helper_dispatch`]), which confines itself with Landlock
//! (filesystem) and seccomp (network) and then executes the command in
//! place, so the process the `ExecutorManager` owns is the command itself.
//! On macOS it is `/usr/bin/sandbox-exec` with a Seatbelt policy, which
//! likewise executes the command in place. On Windows the helper starts the
//! command with a restricted token (Codex's unelevated sandbox) and waits
//! for it, the command in a job that ends with the helper; it is chosen by
//! `[windows] sandbox`. A policy no backend can enforce fails rather than
//! run unconfined.

mod backend;
mod helper;
#[cfg(target_os = "linux")]
mod linux;
mod macos;
mod policy;
mod roots;
mod setup;
#[cfg(any(windows, test))]
mod windows;

pub use backend::{Sandbox, SandboxEnforcement, SandboxError, SandboxStatus};
pub use hanafubuki_agent_foundation::agent_settings::{AgentSandboxMode, AgentWindowsSandbox};
pub use helper::{helper_dispatch, HELPER_ARG, HELPER_FAILURE_EXIT_CODE};
pub use policy::{platform_default_mode, SandboxPolicy, SandboxSettings};
pub use setup::setup_windows_sandbox;
