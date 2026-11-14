//! Wake events: what happened to a session's background work (its
//! sub-agents and its background terminals), queued on the session until a
//! turn delivers them, and what the model reads of them
//! (`<background_events>`).
//!
//! The queue is persisted with the session, so it is decoded one event at a
//! time: an event that does not decode is dropped with a warning, never the
//! rest or the session.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::{AgentExecutionProgressStatus, AgentSessionId};

/// The most `Message` events queued on one session; past it, a new one is
/// refused to its sender.
pub const MAXIMUM_QUEUED_WAKE_MESSAGES: usize = 16;
/// A `Completed` answer is cut at this many bytes (`truncated`); the rest
/// stays in the sub-agent's own conversation.
pub const MAXIMUM_WAKE_ANSWER_BYTES: usize = 32 * 1024;
/// A terminal's output keeps its last this many bytes, from the line they
/// start on.
pub const MAXIMUM_WAKE_OUTPUT_BYTES: usize = 4 * 1024;
/// A terminal's command is cut at this many bytes.
pub const MAXIMUM_WAKE_COMMAND_BYTES: usize = 1024;

/// The kind of the context fragment a `WakeEvents` item is lowered to.
pub(crate) const BACKGROUND_EVENTS_FRAGMENT: &str = "background_events";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWakeEvent {
    /// Per session, in the order the events were queued.
    pub sequence: u64,
    /// Milliseconds since the Unix epoch.
    pub queued_at: u64,
    pub source: AgentWakeSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentWakeSource {
    Subagent {
        session_id: AgentSessionId,
        /// Its task name.
        name: String,
        event: AgentSubagentWake,
    },
    Terminal {
        /// The model's `session_id` for it.
        execution_id: u64,
        command: String,
        event: AgentTerminalWake,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentSubagentWake {
    Completed {
        #[serde(default)]
        answer: Option<String>,
        #[serde(default)]
        truncated: bool,
    },
    Errored {
        error: String,
    },
    ClosedByUser,
    /// Stopped by a restart of the Agent.
    Lost,
    Message {
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentTerminalWake {
    Ended {
        status: AgentExecutionProgressStatus,
        #[serde(default)]
        exit_code: Option<i32>,
        #[serde(default)]
        duration_millis: u64,
        /// Its last output, plain text.
        #[serde(default)]
        output: String,
        #[serde(default)]
        truncated: bool,
    },
    StoppedByUser {
        #[serde(default)]
        exit_code: Option<i32>,
    },
    Lost,
}

/// Whether a session's wake events are delivered (D5). With it off, no
/// admission or step drains the queue, and only a sub-agent's final status
/// is queued.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWakePolicy {
    #[default]
    Off,
    On,
}

/// What became of an event given to the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentWakeQueued {
    /// A turn runs: its next step delivers it.
    Delivering,
    /// No turn runs: the session's next turn delivers it.
    Waiting,
    /// The policy is off and the event is not a sub-agent's final status:
    /// it was dropped.
    Disabled,
}

/// Which queued events `take_wake_events` removes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentWakeFilter {
    /// Every sub-agent's final status (not its messages): what `wait_agent`
    /// reads.
    SubagentStatuses,
    /// The end of one terminal, which a poll that saw it withdraws.
    Terminal { execution_id: u64 },
}

impl AgentWakeFilter {
    fn matches(self, source: &AgentWakeSource) -> bool {
        match (self, source) {
            (Self::SubagentStatuses, AgentWakeSource::Subagent { event, .. }) => {
                !matches!(event, AgentSubagentWake::Message { .. })
            }
            (
                Self::Terminal { execution_id },
                AgentWakeSource::Terminal {
                    execution_id: queued,
                    ..
                },
            ) => execution_id == *queued,
            _ => false,
        }
    }
}

impl AgentWakeSource {
    /// A sub-agent's final status, which is queued even with the policy off.
    pub(crate) fn is_subagent_status(&self) -> bool {
        AgentWakeFilter::SubagentStatuses.matches(self)
    }

    /// Whether `other` is the same sub-agent's or terminal's final event,
    /// which a newer one replaces. Messages are never replaced.
    fn replaces(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Subagent {
                    session_id, event, ..
                },
                Self::Subagent {
                    session_id: queued,
                    event: queued_event,
                    ..
                },
            ) => {
                session_id == queued
                    && !matches!(event, AgentSubagentWake::Message { .. })
                    && !matches!(queued_event, AgentSubagentWake::Message { .. })
            }
            (
                Self::Terminal { execution_id, .. },
                Self::Terminal {
                    execution_id: queued,
                    ..
                },
            ) => execution_id == queued,
            _ => false,
        }
    }

    pub(crate) fn is_message(&self) -> bool {
        matches!(
            self,
            Self::Subagent {
                event: AgentSubagentWake::Message { .. },
                ..
            }
        )
    }

    /// The source within its bounds: an answer and a command cut, an output
    /// kept from its end.
    fn bounded(mut self) -> Self {
        match &mut self {
            Self::Subagent {
                event:
                    AgentSubagentWake::Completed {
                        answer: Some(answer),
                        truncated,
                    },
                ..
            } => *truncated |= cut_end(answer, MAXIMUM_WAKE_ANSWER_BYTES),
            Self::Subagent { .. } => {}
            Self::Terminal { command, event, .. } => {
                cut_end(command, MAXIMUM_WAKE_COMMAND_BYTES);
                if let AgentTerminalWake::Ended {
                    output, truncated, ..
                } = event
                {
                    *truncated |= keep_tail(output, MAXIMUM_WAKE_OUTPUT_BYTES);
                }
            }
        }
        self
    }
}

/// A session's wake events not yet delivered, and the last sequence it gave.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWakeQueue {
    pub events: Vec<AgentWakeEvent>,
    pub last_sequence: u64,
}

impl AgentWakeQueue {
    /// A session that never queued an event: its record leaves the queue out.
    pub fn is_unused(&self) -> bool {
        self.last_sequence == 0 && self.events.is_empty()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Whether another `Message` would pass the bound.
    pub(crate) fn messages_full(&self) -> bool {
        self.events
            .iter()
            .filter(|event| event.source.is_message())
            .count()
            >= MAXIMUM_QUEUED_WAKE_MESSAGES
    }

    /// Queues `source`, within its bounds, at the end; the same sub-agent's
    /// or terminal's final event already queued is replaced.
    pub(crate) fn push(&mut self, source: AgentWakeSource, queued_at: u64) {
        let source = source.bounded();
        let sequence = self.last_sequence.saturating_add(1);
        self.events
            .retain(|queued| !source.replaces(&queued.source));
        self.events.push(AgentWakeEvent {
            sequence,
            queued_at,
            source,
        });
        self.last_sequence = sequence;
    }

    /// Removes and returns the events `filter` matches, in queue order.
    pub(crate) fn take(&mut self, filter: AgentWakeFilter) -> Vec<AgentWakeEvent> {
        let (taken, kept) = std::mem::take(&mut self.events)
            .into_iter()
            .partition(|event| filter.matches(&event.source));
        self.events = kept;
        taken
    }
}

impl<'de> Deserialize<'de> for AgentWakeQueue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Persisted {
            #[serde(default, deserialize_with = "tolerant_wake_events")]
            events: Vec<AgentWakeEvent>,
            #[serde(default, deserialize_with = "tolerant_sequence")]
            last_sequence: u64,
        }
        let value = Value::deserialize(deserializer)?;
        let persisted = match serde_json::from_value::<Persisted>(value) {
            Ok(persisted) => persisted,
            Err(error) => {
                log::warn!("agent wake queue dropped: {error}");
                return Ok(Self::default());
            }
        };
        // A counter that went missing still gives a new event a later
        // sequence than every queued one.
        let last_sequence = persisted
            .events
            .iter()
            .map(|event| event.sequence)
            .fold(persisted.last_sequence, u64::max);
        Ok(Self {
            events: persisted.events,
            last_sequence,
        })
    }
}

/// A counter that does not decode is none; the queue's events give it back.
fn tolerant_sequence<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    Ok(u64::deserialize(Value::deserialize(deserializer)?).unwrap_or_default())
}

/// Persisted wake events, one at a time: an event that does not decode (an
/// unknown kind, a field of the wrong type) is dropped with a warning.
pub(crate) fn tolerant_wake_events<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<AgentWakeEvent>, D::Error> {
    let values = match Value::deserialize(deserializer)? {
        Value::Array(values) => values,
        other => {
            log::warn!("agent wake events dropped: not a list ({other})");
            return Ok(Vec::new());
        }
    };
    Ok(values
        .into_iter()
        .filter_map(|value| match serde_json::from_value(value) {
            Ok(event) => Some(event),
            Err(error) => {
                log::warn!("agent wake event dropped: {error}");
                None
            }
        })
        .collect())
}

impl AgentWakeSource {
    /// One line of what happened, for a person: the event, with the first
    /// line of its answer, error, message or output.
    pub fn summary(&self) -> String {
        let (event, detail) = match self {
            Self::Subagent {
                session_id,
                name,
                event,
            } => {
                let agent = format!("`{name}` (agent id {session_id})");
                match event {
                    AgentSubagentWake::Completed { answer, .. } => {
                        (format!("Sub-agent {agent} completed"), answer.as_deref())
                    }
                    AgentSubagentWake::Errored { error } => {
                        (format!("Sub-agent {agent} failed"), Some(error.as_str()))
                    }
                    AgentSubagentWake::ClosedByUser => {
                        (format!("Sub-agent {agent} was closed by the user"), None)
                    }
                    AgentSubagentWake::Lost => (
                        format!("Sub-agent {agent} stopped when the Agent restarted"),
                        None,
                    ),
                    AgentSubagentWake::Message { text } => (
                        format!("Message from sub-agent {agent}"),
                        Some(text.as_str()),
                    ),
                }
            }
            Self::Terminal {
                execution_id,
                command,
                event,
            } => {
                let session = format!(
                    "Terminal session {execution_id} (`{}`)",
                    first_line(command)
                );
                match event {
                    AgentTerminalWake::Ended { output, .. } => (
                        format!("{session} {}", terminal_end(event)),
                        Some(output.as_str()),
                    ),
                    AgentTerminalWake::StoppedByUser { .. } => {
                        (format!("{session} was stopped by the user"), None)
                    }
                    AgentTerminalWake::Lost => {
                        (format!("{session} stopped when the Agent restarted"), None)
                    }
                }
            }
        };
        match detail.map(first_line).filter(|line| !line.is_empty()) {
            Some(line) => format!("{event}: {}", cut_chars(line, MAXIMUM_SUMMARY_DETAIL_CHARS)),
            None => event,
        }
    }
}

/// A summary's answer, error, message or output line is cut at this many
/// characters.
const MAXIMUM_SUMMARY_DETAIL_CHARS: usize = 200;

fn cut_chars(text: &str, maximum: usize) -> String {
    match text.char_indices().nth(maximum) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

/// What the model reads of `events`: one block each, in queue order.
pub fn wake_events_record(events: &[AgentWakeEvent]) -> String {
    let mut record = String::from(
        "<background_events>\nUpdates on work you started in the background: your sub-agents and your background terminals. The user did not write this.",
    );
    for event in events {
        record.push_str("\n\n");
        record.push_str(&event_block(&event.source));
    }
    record.push_str("\n</background_events>");
    record
}

fn event_block(source: &AgentWakeSource) -> String {
    match source {
        AgentWakeSource::Subagent {
            session_id,
            name,
            event,
        } => subagent_block(&format!("`{name}` (agent id {session_id})"), event),
        AgentWakeSource::Terminal {
            execution_id,
            command,
            event,
        } => terminal_block(*execution_id, command, event),
    }
}

fn subagent_block(agent: &str, event: &AgentSubagentWake) -> String {
    match event {
        AgentSubagentWake::Completed {
            answer: Some(answer),
            truncated,
        } => {
            let cut = if *truncated {
                "… (cut; the rest is in the sub-agent's conversation)"
            } else {
                ""
            };
            format!(
                "Sub-agent {agent} completed. Its final answer:\n<answer>\n{answer}{cut}\n</answer>"
            )
        }
        AgentSubagentWake::Completed { answer: None, .. } => {
            format!("Sub-agent {agent} completed without a final answer.")
        }
        AgentSubagentWake::Errored { error } => format!("Sub-agent {agent} failed: {error}"),
        AgentSubagentWake::ClosedByUser => format!(
            "Sub-agent {agent} was closed by the user: its turn, its terminals and its own sub-agents were stopped. Do not start it again unless the user asks."
        ),
        AgentSubagentWake::Lost => format!(
            "Sub-agent {agent} stopped when the Agent restarted. It keeps its conversation and takes follow-up tasks."
        ),
        AgentSubagentWake::Message { text } => {
            format!("Message from sub-agent {agent}:\n<message>\n{text}\n</message>")
        }
    }
}

fn terminal_block(execution_id: u64, command: &str, event: &AgentTerminalWake) -> String {
    let session = format!("Terminal session {execution_id}");
    let command = format!("<command>\n{command}\n</command>");
    match event {
        AgentTerminalWake::Ended { output, .. } => {
            let mut block = format!("{session} {}:\n{command}", terminal_end(event));
            if !output.is_empty() {
                block.push_str(&format!(
                    "\nIts last output (poll session {execution_id} for the rest):\n<output>\n{output}\n</output>"
                ));
            }
            block
        }
        AgentTerminalWake::StoppedByUser { .. } => {
            format!("{session} was stopped by the user:\n{command}")
        }
        AgentTerminalWake::Lost => {
            format!("{session} stopped when the Agent restarted:\n{command}")
        }
    }
}

/// How a terminal ended, as the record and the summary say it.
fn terminal_end(event: &AgentTerminalWake) -> String {
    let AgentTerminalWake::Ended {
        status,
        exit_code,
        duration_millis,
        ..
    } = event
    else {
        return String::new();
    };
    let seconds = duration_millis / 1000;
    match (status, exit_code) {
        (AgentExecutionProgressStatus::Failed, _) => {
            format!("failed to run after {seconds} seconds")
        }
        (AgentExecutionProgressStatus::TimedOut, _) => {
            format!("timed out after {seconds} seconds and was stopped")
        }
        (AgentExecutionProgressStatus::Cancelled, _) => {
            format!("was stopped after {seconds} seconds")
        }
        (_, Some(code)) => format!("exited with code {code} after {seconds} seconds"),
        (_, None) => format!("exited after {seconds} seconds"),
    }
}

/// Cuts `text` to at most `maximum` bytes at a character boundary; whether
/// it cut.
fn cut_end(text: &mut String, maximum: usize) -> bool {
    if text.len() <= maximum {
        return false;
    }
    let mut end = maximum;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

/// Keeps the last `maximum` bytes of `text`, from the line they start on
/// (or the first character after the cut when no line starts near it);
/// whether it cut.
fn keep_tail(text: &mut String, maximum: usize) -> bool {
    if text.len() <= maximum {
        return false;
    }
    let cut = text.len() - maximum;
    let start = hanafubuki_agent_foundation::terminal_text::aligned_start(text.as_bytes(), cut);
    let mut start = start.min(text.len());
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text.drain(..start);
    true
}

#[cfg(test)]
mod tests;
