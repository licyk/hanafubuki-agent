//! Request / response types for the command executor.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::terminal_size::{TerminalResize, TerminalSize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutorStatus {
    Starting,
    Running,
    Exited,
    Failed,
    Cancelled,
    TimedOut,
}

impl ExecutorStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, ExecutorStatus::Starting | ExecutorStatus::Running)
    }
}

/// Which stream an output chunk came from. PTY output is `Merged` (a PTY combines
/// stdout+stderr); pipe output preserves `Stdout` / `Stderr`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputStream {
    Stdout,
    Stderr,
    Merged,
}

fn default_true() -> bool {
    true
}

/// Selects the executor backend and its I/O semantics.
///
/// Pipe mode is the non-interactive default: stdout/stderr remain distinct and
/// stdin is closed unless explicitly streamed. PTY mode provides terminal
/// semantics, merged output, resizing, and an open input writer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorIoMode {
    #[default]
    Pipe,
    Pty,
}

impl ExecutorIoMode {
    pub const fn uses_pty(self) -> bool {
        matches!(self, Self::Pty)
    }
}

/// Whether the child starts from the desktop process environment or from an
/// empty environment. Callers must choose explicitly at every spawn boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentInheritance {
    Inherit,
    Clear,
}

/// A single command execution request. `command` is **argv** — the executor never
/// parses a shell string; callers pass an explicit shell wrapper when they want
/// shell behavior (e.g. `["sh", "-lc", "echo 123"]`).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutorStartRequest {
    #[serde(default)]
    pub executor_id: Option<String>,
    pub command: Vec<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    pub environment_inheritance: EnvironmentInheritance,
    /// `Some(value)` sets/overrides; `None` unsets the variable. The global
    /// process environment is never mutated.
    #[serde(default)]
    pub env: HashMap<String, Option<String>>,
    #[serde(default)]
    pub io_mode: ExecutorIoMode,
    /// Keep the child's stdin open so it can be written to via `write_stdin`.
    #[serde(default)]
    pub stream_stdin: bool,
    #[serde(default = "default_true")]
    pub stream_output: bool,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub output_bytes_cap: Option<usize>,
}

impl ExecutorStartRequest {
    /// The size a PTY started for this request opens at.
    pub fn terminal_size(&self) -> TerminalSize {
        TerminalSize::new(
            self.cols.unwrap_or(TerminalSize::DEFAULT.cols),
            self.rows.unwrap_or(TerminalSize::DEFAULT.rows),
        )
    }

    /// Codex parity: PTY mode implies both stdin streaming and output streaming. A
    /// PTY spawn always keeps a writer and always captures output, so callers (the
    /// frontend) need not remember to set `stream_stdin`. Pipe
    /// mode is left untouched.
    pub(crate) fn normalize_io_mode(&mut self) {
        if self.io_mode.uses_pty() {
            self.stream_stdin = true;
            self.stream_output = true;
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutorInfo {
    pub executor_id: String,
    pub status: ExecutorStatus,
    pub io_mode: ExecutorIoMode,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub output_offset: u64,
}

/// A contiguous run of same-stream bytes returned by a read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDelta {
    pub stream: OutputStream,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutorOutput {
    pub executor_id: String,
    pub status: ExecutorStatus,
    pub exit_code: Option<i32>,
    pub chunks: Vec<OutputDelta>,
    pub start_offset: u64,
    pub next_offset: u64,
    pub truncated: bool,
    /// PTY size the bytes at `start_offset` were produced at; `None` for pipes.
    pub terminal_size: Option<TerminalSize>,
    /// PTY size changes inside the returned range, in offset order.
    pub resizes: Vec<TerminalResize>,
}
