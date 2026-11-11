//! What `exec` prints: the turn as text for a person, or each event as one
//! JSON object per line for a program (site/content/docs/json-events.en.mdx
//! has the schema).

use std::collections::HashMap;
use std::io::Write;

use hanafubuki_agent::{
    AgentEvent, AgentEventKind, AgentMessageId, AgentMessagePhase, AgentSessionId,
    AgentTurnTerminalReason,
};

pub(super) trait Output {
    /// The session the turn runs in.
    fn session(&mut self, id: AgentSessionId);
    fn event(&mut self, event: &AgentEvent);
    /// Something `exec` itself did or saw (an approval it answered).
    fn note(&mut self, note: &str);
    /// The turn's final message, once it completed.
    fn final_message(&self) -> Option<String>;
}

/// The text of the turn's messages, from their deltas.
#[derive(Default)]
struct Messages {
    text: HashMap<AgentMessageId, String>,
    final_message: Option<String>,
}

impl Messages {
    fn record(&mut self, kind: &AgentEventKind) {
        match kind {
            AgentEventKind::AssistantTextDelta { message_id, delta }
            | AgentEventKind::ProposedPlanDelta { message_id, delta } => {
                self.text.entry(*message_id).or_default().push_str(delta)
            }
            AgentEventKind::TurnCompleted {
                final_answer_message_id,
            } => self.final_message = self.text.get(final_answer_message_id).cloned(),
            _ => {}
        }
    }
}

/// The turn for a person: messages and tool calls on standard output,
/// errors, the session and how long the model thought on standard error.
#[derive(Default)]
pub(super) struct Human {
    messages: Messages,
    tools: HashMap<String, String>,
}

impl Output for Human {
    fn session(&mut self, id: AgentSessionId) {
        eprintln!("session {id}");
    }

    fn event(&mut self, event: &AgentEvent) {
        self.messages.record(&event.kind);
        match &event.kind {
            AgentEventKind::ReasoningCompleted {
                duration_millis, ..
            } => eprintln!("· {}", thought_for(*duration_millis)),
            AgentEventKind::AssistantMessageCompleted { message_id, phase } => {
                let text = self.messages.text.get(message_id).map(String::as_str);
                let text = text.unwrap_or_default().trim_end();
                match phase {
                    Some(AgentMessagePhase::Commentary) => println!("· {text}"),
                    _ => println!("{text}"),
                }
            }
            AgentEventKind::ProposedPlanCompleted { message_id } => {
                let plan = self.messages.text.get(message_id).map(String::as_str);
                println!("plan:\n{}", plan.unwrap_or_default().trim_end());
            }
            AgentEventKind::ToolStarted {
                call_id,
                name,
                presentation,
            } => {
                self.tools.insert(call_id.clone(), name.clone());
                match &presentation.subject {
                    Some(subject) => println!("→ {name}: {subject}"),
                    None => println!("→ {name}"),
                }
            }
            AgentEventKind::ToolCompleted { call_id, is_error } => {
                let name = self.tools.get(call_id).map(String::as_str);
                let result = if *is_error { "failed" } else { "done" };
                println!("  {} {result}", name.unwrap_or("tool"));
            }
            AgentEventKind::ContextCompressed {
                blocks_created,
                tokens_compressed,
                tier,
                ..
            } => println!(
                "  folded into {blocks_created} block(s), ~{tokens_compressed} tokens (tier {tier})"
            ),
            AgentEventKind::ContextCompressionFailed { message } => eprintln!(
                "warning: context compression failed; the turn goes on with the full history: {message}"
            ),
            AgentEventKind::ModelContextWindowLearned { tokens, .. } => eprintln!(
                "the Provider refused the request as too long; this model's window is taken as {tokens} from now on"
            ),
            AgentEventKind::ModelRetryScheduled {
                current_retry,
                maximum_retries,
                reason,
                ..
            } => eprintln!(
                "retrying the model request ({current_retry}/{maximum_retries}): {reason}"
            ),
            AgentEventKind::TurnFailed { reason, message } => {
                eprintln!("error: the turn failed ({}): {message}", label(reason))
            }
            AgentEventKind::TurnCancelled { reason } => {
                eprintln!("the turn was cancelled ({})", label(reason))
            }
            _ => {}
        }
        let _ = std::io::stdout().flush();
    }

    fn note(&mut self, note: &str) {
        println!("· {note}");
    }

    fn final_message(&self) -> Option<String> {
        self.messages.final_message.clone()
    }
}

/// Every event as the facade serializes it, one per line on standard
/// output; `exec`'s own notes go to standard error.
#[derive(Default)]
pub(super) struct JsonLines {
    messages: Messages,
}

impl Output for JsonLines {
    fn session(&mut self, _id: AgentSessionId) {}

    fn event(&mut self, event: &AgentEvent) {
        self.messages.record(&event.kind);
        match serde_json::to_string(event) {
            Ok(line) => {
                let mut stdout = std::io::stdout().lock();
                let _ = writeln!(stdout, "{line}");
                let _ = stdout.flush();
            }
            Err(error) => eprintln!("error: an event could not be written: {error}"),
        }
    }

    fn note(&mut self, note: &str) {
        eprintln!("{note}");
    }

    fn final_message(&self) -> Option<String> {
        self.messages.final_message.clone()
    }
}

/// How long a reasoning item took, as `exec` and `sessions show` print it.
pub(crate) fn thought_for(duration_millis: u64) -> String {
    format!("thought for {}s", duration_millis / 1000)
}

/// A terminal reason as it is serialized (`model_error`).
fn label(reason: &AgentTurnTerminalReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}
