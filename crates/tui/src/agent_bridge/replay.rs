//! A session's committed history as the chat shows it: the transcript a
//! reopened session replays, and what a call that just finished shows (tool
//! events carry neither its output nor its arguments). A call is shown as
//! its tool's presenter says (`AgentToolCallDisplay`).

use std::collections::HashMap;

use hanafubuki_agent::{
    AgentHandle, AgentRevision, AgentSessionId, AgentSessionMode, AgentToolCallDisplay,
    AgentToolDisplay, ConversationItem,
};

use super::{AgentBridge, AgentReply};
use crate::app_event::{AppEvent, AppEventSender};

/// What the interface shows from now on: a reopened session, or no
/// session yet (at the start, after `/new`), which the next message creates.
#[derive(Debug)]
pub(crate) struct OpenedSession {
    pub(crate) session_id: Option<AgentSessionId>,
    /// The session's model; with none yet, the one it will get.
    pub(crate) model: String,
    pub(crate) mode: AgentSessionMode,
    /// Its latest transcript items, oldest first; empty for a new session.
    pub(crate) transcript: Vec<ReplayedItem>,
    /// Where the items left out begin, when older items were left out.
    pub(crate) older: Option<OlderTranscript>,
    /// With no session: a Provider and a model must be set up before the
    /// first message can run (`setup_needed`).
    pub(crate) setup_needed: bool,
}

/// The page of a session's transcript before the ones read so far.
#[derive(Clone, Debug)]
pub(crate) struct OlderTranscript {
    pub(crate) session_id: AgentSessionId,
    pub(crate) cursor: String,
    /// The revision the first page was read at, so later pages read the
    /// same history.
    pub(crate) through_revision: AgentRevision,
}

/// One entry of a reopened session's transcript.
#[derive(Debug)]
pub(crate) enum ReplayedItem {
    User(String),
    /// What the model thought, as Markdown, and for how long.
    Reasoning {
        /// Its history item id, which names its entry.
        item_id: u64,
        text: String,
        duration_millis: u64,
        truncated: bool,
    },
    /// An assistant message's markdown.
    Assistant(String),
    /// A proposed plan's markdown.
    Plan(String),
    Tool {
        call_id: String,
        name: String,
        subject: Option<String>,
        is_error: bool,
        /// What its output shows; `None` without an output.
        display: Option<AgentToolDisplay>,
    },
    Compacted,
}

/// The transcript items a person reads, oldest first, from a page of
/// `items` (with their history item ids) read latest first. A call's output
/// fills in its call; an output whose call is older than the page is left
/// out.
pub(crate) fn transcript(
    agent: &AgentHandle,
    items: Vec<(u64, ConversationItem)>,
) -> Vec<ReplayedItem> {
    let mut transcript = Vec::new();
    // Each call's name and arguments, which its output is shown with.
    let mut calls = HashMap::new();
    for (item_id, content) in items.into_iter().rev() {
        match content {
            ConversationItem::UserMessage { text, .. }
            | ConversationItem::SteeringMessage { text, .. } => {
                transcript.push(ReplayedItem::User(text))
            }
            ConversationItem::Reasoning {
                text,
                duration_millis,
                truncated,
                ..
            } => transcript.push(ReplayedItem::Reasoning {
                item_id,
                text,
                duration_millis,
                truncated,
            }),
            ConversationItem::AssistantMessage {
                text,
                proposed_plan,
                ..
            } => {
                if !text.trim().is_empty() {
                    transcript.push(ReplayedItem::Assistant(text));
                }
                if let Some(plan) = proposed_plan {
                    transcript.push(ReplayedItem::Plan(plan.text));
                }
            }
            ConversationItem::ToolCall {
                call_id,
                namespace,
                name,
                arguments,
                ..
            } => {
                let shown = agent.present_tool_call(namespace.as_deref(), &name, &arguments, None);
                transcript.push(ReplayedItem::Tool {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    subject: shown.subject,
                    is_error: false,
                    display: None,
                });
                calls.insert(call_id, (namespace, name, arguments));
            }
            ConversationItem::ToolOutput {
                call_id, output, ..
            } => {
                let Some((namespace, name, arguments)) = calls.get(&call_id) else {
                    continue;
                };
                let shown =
                    agent.present_tool_call(namespace.as_deref(), name, arguments, Some(&output));
                let entry = transcript.iter_mut().rev().find_map(|entry| match entry {
                    ReplayedItem::Tool {
                        call_id: id,
                        is_error,
                        display,
                        ..
                    } if *id == call_id => Some((is_error, display)),
                    _ => None,
                });
                if let Some((is_error, display)) = entry {
                    *is_error = output.is_error;
                    *display = shown.display;
                }
            }
            ConversationItem::CompactionSummary { .. } => transcript.push(ReplayedItem::Compacted),
            _ => {}
        }
    }
    transcript
}

impl AgentBridge {
    /// Reads what the finished call `call_id` shows on a blocking thread
    /// of the Agent's runtime, and replies with it. A reply always comes,
    /// with no display when the read failed or never ran, so the call's
    /// entry is never left waiting.
    pub(super) fn read_tool_display(&self, session_id: AgentSessionId, call_id: String) {
        let agent = self.agent.clone();
        let mut reply = DisplayReply {
            app_event_tx: self.app_event_tx.clone(),
            session_id,
            call_id,
            display: None,
        };
        // A runtime that has shut down drops the closure, and the reply
        // with it.
        drop(self.agent.runtime().handle().spawn_blocking(move || {
            let display = agent.tool_display(reply.session_id, &reply.call_id);
            reply.shows(display.ok().flatten());
        }));
    }
}

/// Sent when dropped: after the read, or in place of one.
struct DisplayReply {
    app_event_tx: AppEventSender,
    session_id: AgentSessionId,
    call_id: String,
    display: Option<AgentToolCallDisplay>,
}

impl DisplayReply {
    fn shows(&mut self, display: Option<AgentToolCallDisplay>) {
        self.display = display;
    }
}

impl Drop for DisplayReply {
    fn drop(&mut self) {
        self.app_event_tx
            .send(AppEvent::AgentReply(AgentReply::ToolDisplayed {
                session_id: self.session_id,
                call_id: std::mem::take(&mut self.call_id),
                display: self.display.take(),
            }));
    }
}

#[cfg(test)]
mod tests;
