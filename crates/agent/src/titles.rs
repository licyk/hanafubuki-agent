//! Automatic session titles: the provisional title the submit command gives
//! a session that has none (immediate feedback, committed with the turn), and
//! the narrow port through which the `titles` plugin reads what it needs to
//! generate a better one and commits it.

use hanafubuki_agent_config::AgentConfig;
use hanafubuki_agent_kernel::{AgentService, AgentSessionId};

use crate::conversation_reader::{AgentConversationReader, AgentSessionModel};

/// The longest automatic title, provisional or generated, in characters.
pub(super) const AUTOMATIC_AGENT_TITLE_MAX_CHARS: usize = 36;

/// The submitted text with its whitespace collapsed, cut to the automatic
/// bound; `None` for a text without words.
pub(super) fn provisional_agent_title(prompt: &str) -> Option<String> {
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    Some(
        normalized
            .chars()
            .take(AUTOMATIC_AGENT_TITLE_MAX_CHARS)
            .collect(),
    )
}

/// What the `titles` plugin reads and writes: the session's first message
/// and its Provider, the conversation a suggestion reads, and the
/// compare-and-swap that commits a generated title. Every method blocks;
/// the plugin calls them on the `BlockingService`.
#[derive(Clone)]
pub(crate) struct AgentTitleApplication {
    core: AgentService,
    config: AgentConfig,
}

/// Everything one title generation needs.
pub(crate) struct AgentTitleRequest {
    /// The session's first user message, which the title is made from.
    pub(crate) prompt: String,
    pub(crate) model: AgentSessionModel,
}

impl AgentTitleApplication {
    pub(crate) fn new(core: AgentService, config: AgentConfig) -> Self {
        Self { core, config }
    }

    /// The session's first user message and its model on its Provider as
    /// the settings now describe it, while `title` is still that message's
    /// provisional title; `None` once it is not (a title generated earlier
    /// whose commit was lost, a session titled otherwise).
    pub(crate) fn title_request(
        &self,
        session_id: AgentSessionId,
        title: &str,
    ) -> Result<Option<AgentTitleRequest>, String> {
        let Some(prompt) = self
            .conversations()
            .first_user_message(session_id)
            .map_err(|error| error.to_string())?
            .filter(|prompt| provisional_agent_title(prompt).as_deref() == Some(title))
        else {
            return Ok(None);
        };
        let session = self
            .core
            .snapshot(session_id)
            .map_err(|error| error.to_string())?;
        Ok(Some(AgentTitleRequest {
            prompt,
            model: AgentSessionModel::resolve(&self.config, &session, &self.core)?,
        }))
    }

    /// What a title suggestion reads of a session.
    pub(crate) fn conversations(&self) -> AgentConversationReader {
        AgentConversationReader::new(self.core.clone(), self.config.clone())
    }

    /// Replaces the session's automatic title `expected` with `title`.
    /// Returns whether it did: a title the user gave the session in the
    /// meantime is kept.
    pub(crate) fn replace_title(
        &self,
        session_id: AgentSessionId,
        expected: &str,
        title: String,
    ) -> Result<bool, String> {
        self.core
            .replace_automatic_session_title_if_expected(session_id, expected, title)
            .map(|replaced| replaced.is_some())
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests;
