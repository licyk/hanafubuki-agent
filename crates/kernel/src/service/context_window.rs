//! A session's context window, resolved again as each turn starts: the
//! model metadata may know its model's window better than when the session
//! was created (a listing that landed later), and the session takes it. A
//! Provider's refusal of a request as too long teaches the route a window,
//! which the session that was refused takes at once.

use hanafubuki_agent_model_api::{
    context_limit_in, ModelMetadataSource, Sourced, PLAUSIBLE_CONTEXT_WINDOW,
};

use super::model_facts::configured_facts;
use super::*;

impl AgentService {
    /// Gives `record`'s session the window its route resolves to now, and
    /// measures its usage against it too, since recovery refuses usage
    /// measured against another window. Called under the session's lock,
    /// in a commit: the one that admits a turn, or the one after a refusal
    /// taught a window. A configured window wins; a window no longer known
    /// is kept, unless it was recorded as configured or learned (the
    /// configuration or the learned fact is gone), and then the session's
    /// window is unknown.
    /// Returns the window the session took (none: unknown), when it
    /// changed.
    pub(super) fn refresh_context_window(
        &self,
        record: &mut SessionRecord,
    ) -> Option<Option<Sourced<u64>>> {
        let config = &record.settings.config;
        // A session not bound to its Provider yet (recovered, its transport
        // unavailable) has no route: the built-in table alone would take
        // the place of what its Provider reported.
        let route = record.settings.model.route_key(&config.model)?;
        let resolved = self.inner.model_metadata.context_window(
            config.configured_facts.context_window_tokens,
            Some(&route),
            &config.model,
            now_millis_u64(),
        );
        let window = match resolved {
            Some(window) => Some(changed_window(config, window)?),
            // Only a window recorded as configured or learned: one without
            // a source (written before sources were kept) is kept.
            None if matches!(
                config.context_window_source,
                Some(ModelMetadataSource::Configured | ModelMetadataSource::Learned)
            ) =>
            {
                None
            }
            None => return None,
        };
        if let Some(usage) = record.runtime.usage.as_mut() {
            usage.context_window_tokens = window.as_ref().map(|window| window.value);
        }
        record.settings.config.set_context_window(window.clone());
        Some(window)
    }

    /// Learns from the Provider's refusal of `request` (with `message`) as
    /// too long for its model's window: the window the refusal names, else
    /// the request's estimated size, an upper bound. The route keeps it
    /// while it is the smallest it was taught, and it is used while it is
    /// smaller than what else is known (`model_metadata`). Learned whether
    /// or not the session compacts automatically; the session takes it at
    /// once, unless its window is configured, and says so with
    /// `ModelContextWindowLearned`.
    pub(super) async fn learn_context_window(
        &self,
        identity: TurnIdentity,
        model: &dyn ModelSession,
        request: &ModelRequest,
        message: &str,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let Some(route) = model.route_key(&request.model) else {
            return Ok(());
        };
        let Some(tokens) = taught_window(request, message) else {
            return Ok(());
        };
        self.inner
            .model_metadata
            .learn_context_window(&route, tokens, now_millis_u64());
        let cell = self.session(identity.session_id)?;
        let (staged, event) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            exact_turn_identity(record, identity)?;
            ensure_no_staged_commit(record)?;
            let rollback = record.rollback_state();
            let previous = record.settings.config.context_window_tokens;
            let Some(window) = self.refresh_context_window(record) else {
                return Ok(());
            };
            record.status.revision = record
                .status
                .revision
                .checked_add(1)
                .expect("agent snapshot revision exhausted");
            record.status.updated_at = now_millis();
            let event = window
                .filter(|window| window.source == ModelMetadataSource::Learned)
                .map(|window| {
                    record_event(
                        record,
                        identity,
                        AgentEventKind::ModelContextWindowLearned {
                            tokens: window.value,
                            previous,
                        },
                        self.inner.limits,
                    )
                });
            (StagedPersistence::commit(&cell, record, rollback), event)
        };
        self.persist_staged(staged).await?;
        if let Some(event) = event {
            event_sink.emit(event);
        }
        Ok(())
    }

    /// The largest output `record`'s model is known to give, which the
    /// proactive compaction leaves room for; none when it is unknown or the
    /// session has no route.
    pub(super) fn max_output_tokens(&self, record: &SessionRecord) -> Option<u64> {
        let config = &record.settings.config;
        let route = record.settings.model.route_key(&config.model)?;
        self.inner
            .model_metadata
            .resolve(
                Some(&route),
                &config.model,
                configured_facts(config),
                now_millis_u64(),
            )
            .max_output_tokens
            .map(|fact| fact.value)
    }
}

/// The window a refusal of `request` with `message` teaches: the one the
/// message names, else the request's estimated size (it did not fit),
/// within the same bounds. A request under them taken for too long is more
/// likely a misread refusal than a model that small.
fn taught_window(request: &ModelRequest, message: &str) -> Option<u64> {
    context_limit_in(message).or_else(|| {
        estimate_model_request_tokens(request)
            .ok()
            .filter(|estimated| PLAUSIBLE_CONTEXT_WINDOW.contains(estimated))
    })
}

/// `resolved` when it differs from `config`'s window or its origin (so a
/// window written without a source, configured, stays so).
fn changed_window(config: &AgentSessionConfig, resolved: Sourced<u64>) -> Option<Sourced<u64>> {
    (config.context_window_tokens != Some(resolved.value)
        || config.context_window_origin() != Some(resolved.source))
    .then_some(resolved)
}

#[cfg(test)]
mod tests;
