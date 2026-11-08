//! Tauri-free Agent-owned argv execution sessions.
//!
//! This crate owns only Agent command metadata and exact process receipts.
//! `hanafubuki-agent-process` remains the sole OS process ownership authority.

#[cfg(any(test, feature = "test-support"))]
mod command;
mod environment;
mod model;
mod resolver;
mod service;
mod shell;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(any(test, feature = "test-support"))]
pub use command::{prepare_command, PreparedCommand};
#[cfg(any(test, feature = "test-support"))]
pub use model::StartCommandRequest;
pub use model::{
    AgentBackgroundTerminalSnapshot, AgentBackgroundTerminalStopFailure,
    AgentBackgroundTerminalStopOutcome, AgentExecutionError, AgentExecutionGeneration,
    AgentExecutionId, AgentExecutionKind, AgentExecutionOutput, AgentExecutionOutputChunk,
    AgentExecutionOutputStream, AgentExecutionScope, AgentExecutionSnapshot, AgentExecutionStatus,
    AgentExecutionWaitOutcome, ConfirmedAgentExecutionDisposal, ExecCommandRequest,
    InteractiveResizeAccepted, PlatformShell, UnifiedExecResult, WriteStdinRequest,
};
pub use service::{AgentExecutionProcessPort, AgentExecutionService, ExecutorAgentExecutionPort};
