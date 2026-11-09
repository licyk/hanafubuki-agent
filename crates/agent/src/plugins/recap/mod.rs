//! The `recap` plugin: a brief catch-up on a session for a user returning
//! to it. It provides [`RecapService`] under its name, which the facade's
//! `recap_session` resolves per call; with the plugin off or failed, only
//! recaps are unavailable.
//!
//! A recap reads the session's latest exchanges of user requests and
//! assistant answers (no tool output, no reasoning), byte-bounded, and asks
//! the session's model for a summary and an optional next action, outside
//! the session's conversation. Nothing is committed.

mod history;
mod prompt;

use std::sync::Arc;

use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{AgentError, AgentSessionId};
use kardia_core::{Context, Disposer, Inject, PluginDef, Service};
use serde::Serialize;

use crate::conversation_reader::AgentConversationReader;
use crate::plugins::structured_request::request_structured;
use history::{recap_history, RecentExchanges};

/// The plugin's name, under which it provides its [`RecapService`].
pub(crate) const RECAP_PLUGIN: &str = "recap";

/// A catch-up on a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecap {
    /// The goal, the progress made and what blocks it, in plain text.
    pub summary: String,
    /// An unanswered question for the user, an agreed next step or the
    /// remedy for the current blocker, if there is one.
    pub next_action: Option<String>,
}

/// Provides the [`RecapService`] over `conversations`.
pub(crate) fn recap_plugin(conversations: AgentConversationReader) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let conversations = conversations.clone();
        async move {
            let blocking = ctx.get::<BlockingService>()?.arc().clone();
            let cancellation = CancellationToken::default();
            let stopping = cancellation.clone();
            ctx.defer("recap requests", Disposer::new(move || stopping.cancel()))?;
            ctx.provide_arc_as(
                RECAP_PLUGIN,
                Arc::new(RecapService {
                    conversations,
                    blocking,
                    cancellation,
                }),
            )?;
            Ok(())
        }
    })
    .name(RECAP_PLUGIN)
    .provides([RECAP_PLUGIN])
    .inject(Inject::new().with(BlockingService::NAME))
}

/// What the `recap` row provides: recaps, cancelled when the plugin stops.
pub(crate) struct RecapService {
    conversations: AgentConversationReader,
    blocking: Arc<BlockingService>,
    cancellation: CancellationToken,
}

impl RecapService {
    /// A recap of `session_id` as it stands.
    pub(crate) async fn recap(&self, session_id: AgentSessionId) -> Result<AgentRecap, AgentError> {
        let conversations = self.conversations.clone();
        let (history, model) = self
            .blocking
            .run("agent recap request", move || {
                let mut exchanges = RecentExchanges::default();
                conversations.visit_messages(session_id, |message| exchanges.push(message))?;
                let history = recap_history(&exchanges.finish());
                if history.is_empty() {
                    return Err(AgentError::InvalidInput(
                        "there is no conversation to recap".to_string(),
                    ));
                }
                Ok((history, conversations.session_model(session_id)?))
            })
            .await
            .map_err(|error| AgentError::Internal(error.to_string()))??;
        let model_session = model.model_session().map_err(AgentError::Model)?;
        let encoded = request_structured(
            model_session,
            prompt::recap_request(model.model, &history),
            self.cancellation.child(),
        )
        .await
        .map_err(AgentError::Model)?;
        prompt::parse_recap(&encoded).map_err(AgentError::Model)
    }
}

#[cfg(test)]
mod tests;
