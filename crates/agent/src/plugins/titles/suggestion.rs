//! A title suggestion for a session the user renames: the service the
//! `titles` row provides under its name. It asks the session's model, as an
//! automatic title does, for a title of the conversation's latest messages,
//! and commits nothing.

use std::ops::ControlFlow;
use std::sync::Arc;

use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{AgentError, AgentSessionId};

use super::generation::{generate_title, TITLE_PROMPT_MAX_BYTES, TITLE_SUGGESTION_INSTRUCTIONS};
use crate::conversation_reader::{AgentConversationReader, ConversationMessage};
use crate::plugins::structured_request::bounded_text;

/// How many of the latest messages a suggestion reads.
const SUGGESTION_MESSAGES: usize = 8;

/// What the `titles` row provides: title suggestions, cancelled when the
/// plugin stops.
pub(crate) struct TitleService {
    pub(super) conversations: AgentConversationReader,
    pub(super) blocking: Arc<BlockingService>,
    pub(super) cancellation: CancellationToken,
}

impl TitleService {
    /// A title for `session_id`'s conversation as it stands.
    pub(crate) async fn suggest(&self, session_id: AgentSessionId) -> Result<String, AgentError> {
        let conversations = self.conversations.clone();
        let (messages, model) = self
            .blocking
            .run("agent title suggestion request", move || {
                let mut messages = Vec::new();
                conversations.visit_messages(session_id, |message| {
                    messages.push(message);
                    if messages.len() == SUGGESTION_MESSAGES {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                })?;
                Ok::<_, AgentError>((messages, conversations.session_model(session_id)?))
            })
            .await
            .map_err(|error| AgentError::Internal(error.to_string()))??;
        if messages.is_empty() {
            return Err(AgentError::InvalidInput(
                "the conversation has no messages to title".to_string(),
            ));
        }
        let model_session = model.model_session().map_err(AgentError::Model)?;
        generate_title(
            model_session,
            model.model,
            TITLE_SUGGESTION_INSTRUCTIONS,
            conversation_excerpt(messages),
            self.cancellation.child(),
        )
        .await
        .map_err(AgentError::Model)
    }
}

/// `newest_first` oldest first, one labelled paragraph each, every message
/// cut to an equal share of the title prompt's bound.
fn conversation_excerpt(mut newest_first: Vec<ConversationMessage>) -> String {
    newest_first.reverse();
    let share = (TITLE_PROMPT_MAX_BYTES / newest_first.len()).saturating_sub("\n\n".len());
    newest_first
        .iter()
        .map(|message| {
            let paragraph = match message {
                ConversationMessage::User(text) => format!("User: {}", text.trim()),
                ConversationMessage::Assistant(text) => format!("Assistant: {}", text.trim()),
            };
            bounded_text(&paragraph, share).to_string()
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests;
