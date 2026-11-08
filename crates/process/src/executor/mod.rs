//! Codex-style command executor.
//!
//! Executes a standalone process from an **argv** vector (never a raw shell
//! string). Pipe mode (default) uses `std::process` with separate stdout/stderr;
//! PTY mode uses `portable-pty` with merged output when terminal behavior is
//! needed. Each execution is an independent session with realtime (offset-based)
//! output, a final exit code, cancellation, and process-tree cleanup. It is
//! entirely separate from the user-facing interactive terminal — no prompt state,
//! shell integration, `^U` clearing, or command-text injection.
//!
//! Core lifecycle logic lives in ordinary Rust functions ([`manager`]). Tauri
//! command adapters and cross-domain mutation authorization live outside this
//! module in `executor_commands`.

mod backend_common;
pub mod manager;
pub mod model;
mod output;
mod pipe;
mod pty;
mod session;
mod spawn_retry;
/// Windows ConPTY backend (Codex-derived), used by the PTY backend on Windows
/// instead of `portable_pty::native_pty_system()`.
#[cfg(windows)]
mod win;
mod windows_input;

pub use manager::ExecutorManager;
pub use model::{
    EnvironmentInheritance, ExecutorInfo, ExecutorIoMode, ExecutorOutput, ExecutorStartRequest,
    ExecutorStatus, OutputDelta, OutputStream,
};

#[cfg(all(test, unix))]
mod tests;
