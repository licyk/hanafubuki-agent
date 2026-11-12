//! What a side request (a title suggestion, a recap) reads of a session:
//! its user and assistant messages, newest first, and its model on its
//! Provider. The narrow port the `titles` and `recap` plugins reach the
//! kernel's sessions through; every method blocks, so the plugins call
//! them on the `BlockingService`.

use std::ops::ControlFlow;
use std::sync::Arc;

use hanafubuki_agent_config::AgentConfig;
use hanafubuki_agent_kernel::{
    AgentError, AgentHistorySortDirection, AgentService, AgentSessionId,
    AgentSessionRuntimeSnapshot, AgentTranscriptPageRequest, ConversationItem, ModelApiProtocol,
    ModelSession, AGENT_TRANSCRIPT_PAGE_SIZE_MAX,
};

use crate::service::resolve_settings_provider;
use crate::{model_api_protocol, ResolvedAgentProvider};

/// One message of a session's conversation as a side request reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConversationMessage {
    /// A user's message or steering.
    User(String),
    /// What the model answered, without its reasoning.
    Assistant(String),
}

#[derive(Clone)]
pub(crate) struct AgentConversationReader {
    core: AgentService,
    config: AgentConfig,
}

impl AgentConversationReader {
    pub(crate) fn new(core: AgentService, config: AgentConfig) -> Self {
        Self { core, config }
    }

    /// Hands `visit` the session's messages, newest first, until it breaks
    /// or the conversation's start; tool calls and their output, reasoning
    /// and context are skipped.
    pub(crate) fn visit_messages(
        &self,
        session_id: AgentSessionId,
        mut visit: impl FnMut(ConversationMessage) -> ControlFlow<()>,
    ) -> Result<(), AgentError> {
        self.core.activate_session(session_id)?;
        let mut cursor = None;
        let mut through_revision = None;
        loop {
            let page = self.core.read_transcript_page(AgentTranscriptPageRequest {
                session_id,
                page_size: AGENT_TRANSCRIPT_PAGE_SIZE_MAX,
                cursor,
                direction: AgentHistorySortDirection::Descending,
                through_revision,
            })?;
            for item in page.items {
                let message = match item.content {
                    ConversationItem::UserMessage { text, .. }
                    | ConversationItem::SteeringMessage { text, .. } => {
                        ConversationMessage::User(text)
                    }
                    ConversationItem::AssistantMessage { text, .. } => {
                        ConversationMessage::Assistant(text)
                    }
                    _ => continue,
                };
                if visit(message).is_break() {
                    return Ok(());
                }
            }
            let Some(older) = page.older_cursor else {
                return Ok(());
            };
            cursor = Some(older);
            // A commit between two pages must not move the window.
            through_revision = Some(page.through_revision);
        }
    }

    /// The session's model on its Provider as the settings now describe it.
    pub(crate) fn session_model(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionModel, AgentError> {
        self.core.activate_session(session_id)?;
        let snapshot = self.core.snapshot(session_id)?;
        if snapshot.model.is_empty() {
            return Err(AgentError::Model(
                "No model is selected for this conversation; use /model or choose one in Agent settings"
                    .to_string(),
            ));
        }
        AgentSessionModel::resolve(&self.config, &snapshot, self.core.model_proxy())
            .map_err(AgentError::Model)
    }
}

/// A session's model on its Provider, for a request outside its
/// conversation.
pub(crate) struct AgentSessionModel {
    pub(crate) model: String,
    protocol: ModelApiProtocol,
    provider: ResolvedAgentProvider,
    proxy: hanafubuki_agent_kernel::ModelHttpProxy,
}

impl AgentSessionModel {
    /// The model `session` runs on its Provider as `config` now describes
    /// it; a Provider whose protocol changed is refused.
    pub(crate) fn resolve(
        config: &AgentConfig,
        session: &AgentSessionRuntimeSnapshot,
        proxy: hanafubuki_agent_kernel::ModelHttpProxy,
    ) -> Result<Self, String> {
        let registry = config.providers()?;
        let provider = resolve_settings_provider(&registry, &session.provider_id)?;
        if model_api_protocol(provider.definition.protocol) != session.provider_protocol {
            return Err(format!(
                "Agent Provider {} protocol changed",
                provider.definition.display_name
            ));
        }
        Ok(Self {
            model: session.model.clone(),
            protocol: session.provider_protocol,
            provider,
            proxy,
        })
    }

    /// A private model session on the session's Provider, outside the
    /// session's own conversation.
    pub(crate) fn model_session(&self) -> Result<Arc<dyn ModelSession>, String> {
        AgentService::create_private_model_session(
            self.protocol,
            &self.provider.definition.base_url,
            self.provider.credential.clone(),
            self.provider.headers.clone(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
            self.proxy.clone(),
        )
        .map_err(|error| error.to_string())
    }
}
