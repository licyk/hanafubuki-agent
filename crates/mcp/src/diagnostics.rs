use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::{McpConnectionGeneration, McpServerId};

pub const MCP_DIAGNOSTIC_CAPACITY: usize = 128;
pub const MCP_DIAGNOSTIC_MESSAGE_BYTES: usize = 2 * 1024;
pub const MCP_STDERR_TAIL_BYTES: usize = 16 * 1024;
pub const MCP_STDERR_TAIL_LINES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpDiagnosticStage {
    ResolveProgram,
    Spawn,
    Initialize,
    DiscoverTools,
    DiscoverResources,
    OAuth,
    ToolCall,
    ToolListChanged,
    Refresh,
    /// The connection's transport closed while it was in use.
    ConnectionLost,
    Shutdown,
    Cleanup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpDiagnosticOutcome {
    Success,
    Warning,
    Error,
    Cancelled,
    OutcomeUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpDiagnosticEntry {
    pub sequence: u64,
    pub timestamp_unix_ms: u64,
    pub stage: McpDiagnosticStage,
    pub outcome: McpDiagnosticOutcome,
    pub code: String,
    pub message: String,
    pub transport: String,
    pub config_generation: [u8; 32],
    pub runtime_generation: Option<McpConnectionGeneration>,
    pub tool_identity: Option<String>,
    pub retryable: bool,
    pub next_step: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpDiagnosticSnapshot {
    pub server_id: McpServerId,
    pub entries: Vec<McpDiagnosticEntry>,
    pub stderr_tail: String,
    pub volatile_until_restart: bool,
}

/// A server's current diagnostics: those recorded since its latest
/// connection attempt began ([`McpDiagnosticStore::begin_attempt`]), or all of
/// them before its first. What an earlier attempt recorded stays in the
/// [`McpDiagnosticSnapshot`] but is not current.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpDiagnosticSummary {
    pub server_id: McpServerId,
    /// Current `Error` and `OutcomeUnknown` entries.
    pub errors: usize,
    /// Current `Warning` entries.
    pub warnings: usize,
    /// The most severe current entry (an error before a warning), the most
    /// recent of its severity; `None` when nothing current is either.
    pub headline: Option<McpDiagnosticEntry>,
    /// Every current entry, oldest first.
    pub entries: Vec<McpDiagnosticEntry>,
}

impl McpDiagnosticSummary {
    fn of(server_id: McpServerId, entries: Vec<McpDiagnosticEntry>) -> Self {
        let count = |level| {
            entries
                .iter()
                .filter(|entry| severity(entry.outcome) == level)
                .count()
        };
        let headline = entries
            .iter()
            .filter(|entry| severity(entry.outcome) > 0)
            .max_by_key(|entry| (severity(entry.outcome), entry.sequence))
            .cloned();
        Self {
            server_id,
            errors: count(2),
            warnings: count(1),
            headline,
            entries,
        }
    }
}

/// 2 for what failed or may have, 1 for a warning, 0 otherwise.
fn severity(outcome: McpDiagnosticOutcome) -> u8 {
    match outcome {
        McpDiagnosticOutcome::Error | McpDiagnosticOutcome::OutcomeUnknown => 2,
        McpDiagnosticOutcome::Warning => 1,
        McpDiagnosticOutcome::Success | McpDiagnosticOutcome::Cancelled => 0,
    }
}

#[derive(Default)]
struct ServerDiagnostics {
    next_sequence: u64,
    /// The sequence of the first current entry.
    current_from: u64,
    entries: VecDeque<McpDiagnosticEntry>,
    stderr_tail: String,
}

#[derive(Clone, Default)]
pub struct McpDiagnosticStore {
    inner: Arc<Mutex<HashMap<McpServerId, ServerDiagnostics>>>,
}

impl McpDiagnosticStore {
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        server_id: McpServerId,
        stage: McpDiagnosticStage,
        outcome: McpDiagnosticOutcome,
        code: impl Into<String>,
        message: impl AsRef<str>,
        transport: impl Into<String>,
        config_generation: [u8; 32],
        runtime_generation: Option<McpConnectionGeneration>,
        tool_identity: Option<String>,
        retryable: bool,
        next_step: impl Into<String>,
    ) {
        let mut registry = self.inner.lock();
        let server = registry.entry(server_id).or_default();
        server.next_sequence = server.next_sequence.saturating_add(1);
        let entry = McpDiagnosticEntry {
            sequence: server.next_sequence,
            timestamp_unix_ms: now_unix_ms(),
            stage,
            outcome,
            code: bounded_safe(&code.into(), 128),
            message: sanitize_mcp_diagnostic(message.as_ref(), MCP_DIAGNOSTIC_MESSAGE_BYTES),
            transport: bounded_safe(&transport.into(), 64),
            config_generation,
            runtime_generation,
            tool_identity: tool_identity.map(|value| bounded_safe(&value, 256)),
            retryable,
            next_step: bounded_safe(&next_step.into(), 128),
        };
        if server.entries.len() == MCP_DIAGNOSTIC_CAPACITY {
            server.entries.pop_front();
        }
        server.entries.push_back(entry);
    }

    /// Starts a connection attempt: what was recorded before it is no longer
    /// current ([`McpDiagnosticSummary`]).
    pub fn begin_attempt(&self, server_id: McpServerId) {
        let mut registry = self.inner.lock();
        let server = registry.entry(server_id).or_default();
        server.current_from = server.next_sequence.saturating_add(1);
    }

    pub fn summary(&self, server_id: &McpServerId) -> McpDiagnosticSummary {
        let entries = self
            .inner
            .lock()
            .get(server_id)
            .map(|server| {
                server
                    .entries
                    .iter()
                    .filter(|entry| entry.sequence >= server.current_from)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        McpDiagnosticSummary::of(server_id.clone(), entries)
    }

    pub fn set_stderr_tail(&self, server_id: McpServerId, stderr: &str) {
        let mut lines = stderr
            .lines()
            .rev()
            .take(MCP_STDERR_TAIL_LINES)
            .collect::<Vec<_>>();
        lines.reverse();
        let sanitized = sanitize_mcp_diagnostic(&lines.join("\n"), MCP_STDERR_TAIL_BYTES);
        self.inner.lock().entry(server_id).or_default().stderr_tail = sanitized;
    }

    pub fn snapshot(&self, server_id: &McpServerId) -> McpDiagnosticSnapshot {
        let registry = self.inner.lock();
        let server = registry.get(server_id);
        McpDiagnosticSnapshot {
            server_id: server_id.clone(),
            entries: server
                .map(|value| value.entries.iter().cloned().collect())
                .unwrap_or_default(),
            stderr_tail: server
                .map(|value| value.stderr_tail.clone())
                .unwrap_or_default(),
            volatile_until_restart: true,
        }
    }
}

pub fn sanitize_mcp_diagnostic(value: &str, maximum: usize) -> String {
    let mut redact_next = false;
    let sanitized = value
        .split_whitespace()
        .map(|word| {
            if redact_next {
                redact_next = false;
                return "[redacted]".to_string();
            }
            let lower = word.to_ascii_lowercase();
            if lower == "bearer" {
                redact_next = true;
                return "Bearer".to_string();
            }
            if lower.starts_with("authorization:bearer") {
                redact_next = true;
                return "Authorization: Bearer".to_string();
            }
            if lower.starts_with("bearer=")
                || lower.starts_with("bearer:")
                || [
                    "token",
                    "access_token",
                    "refresh_token",
                    "password",
                    "secret",
                    "authorization",
                    "code",
                    "verifier",
                ]
                .iter()
                .any(|key| {
                    lower.starts_with(&format!("{key}="))
                        || lower.starts_with(&format!("{key}:"))
                        || lower.starts_with(&format!("\"{key}\":"))
                })
            {
                return "[redacted]".to_string();
            }
            if (lower.starts_with("http://") || lower.starts_with("https://")) && word.contains('?')
            {
                return format!("{}?[redacted]", word.split('?').next().unwrap_or_default());
            }
            word.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ");
    bounded_safe(&sanitized, maximum)
}

fn bounded_safe(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.to_string();
    }
    let boundary = value
        .char_indices()
        .take_while(|(index, _)| *index <= maximum)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0);
    value[..boundary].to_string()
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
