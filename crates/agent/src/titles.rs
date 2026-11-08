//! Automatic session titles: the provisional title the submit command gives
//! a session that has none (immediate feedback, committed with the turn), and
//! the narrow port through which the `titles` plugin reads what it needs to
//! generate a better one and commits it.

use std::sync::Arc;

use hanafubuki_agent_config::AgentConfig;
use hanafubuki_agent_kernel::{
    AgentService, AgentSessionId, AgentTurnId, ModelApiProtocol, ModelSession,
};

use crate::service::resolve_settings_provider;
use crate::{model_api_protocol, ResolvedAgentProvider};

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

/// What the `titles` plugin reads and writes: a turn's prompt and the
/// session's Provider, and the compare-and-swap that commits a generated
/// title. Every method blocks; the plugin calls them on the `BlockingService`.
#[derive(Clone)]
pub(crate) struct AgentTitleApplication {
    core: AgentService,
    config: AgentConfig,
}

/// Everything one title generation needs.
pub(crate) struct AgentTitleRequest {
    pub(crate) prompt: String,
    pub(crate) model: String,
    protocol: ModelApiProtocol,
    provider: ResolvedAgentProvider,
}

impl AgentTitleRequest {
    /// A private model session on the session's Provider, outside the
    /// session's own conversation.
    pub(crate) fn model_session(&self) -> Result<Arc<dyn ModelSession>, String> {
        AgentService::create_private_model_session(
            self.protocol,
            &self.provider.definition.base_url,
            self.provider.credential.clone(),
            self.provider.headers.clone(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
        )
        .map_err(|error| error.to_string())
    }
}

impl AgentTitleApplication {
    pub(crate) fn new(core: AgentService, config: AgentConfig) -> Self {
        Self { core, config }
    }

    /// The user message that started `turn_id` and the session's model on
    /// its Provider as the settings now describe it. `None` when the turn's
    /// message is no longer in the session's history.
    pub(crate) fn title_request(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
    ) -> Result<Option<AgentTitleRequest>, String> {
        let Some(prompt) = self
            .core
            .turn_user_message(session_id, turn_id)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let session = self
            .core
            .snapshot(session_id)
            .map_err(|error| error.to_string())?;
        let registry = self.config.providers()?;
        let provider = resolve_settings_provider(&registry, &session.provider_id)?;
        if model_api_protocol(provider.definition.protocol) != session.provider_protocol {
            return Err(format!(
                "Agent Provider {} protocol changed",
                provider.definition.display_name
            ));
        }
        Ok(Some(AgentTitleRequest {
            prompt,
            model: session.model,
            protocol: session.provider_protocol,
            provider,
        }))
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
