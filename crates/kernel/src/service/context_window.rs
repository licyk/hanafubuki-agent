//! A session's context window, resolved again when a listing of its
//! Provider is recorded, as each turn starts and at each response's
//! commit: the model metadata may know its model's window better than when
//! the session was created (a listing that landed later), and the session
//! takes it. A Provider's refusal of a request as too long teaches the
//! route a window, which the session that was refused takes at once.

use hanafubuki_agent_model_api::{
    context_limit_in, ModelEndpointKey, ModelMetadataSource, Sourced, PLAUSIBLE_CONTEXT_WINDOW,
};

use super::model_facts::configured_facts;
use super::*;

impl AgentService {
    /// Gives every loaded session on `endpoint` the window its route
    /// resolves to now that a listing of `endpoint` was recorded, each in a
    /// commit of its own. A session with a staged commit is skipped: its
    /// next response's commit or turn takes it. A session that cannot take
    /// it (removed meanwhile, its commit refused) keeps what it had, and the
    /// others still take theirs.
    pub(super) async fn take_listed_context_windows(&self, endpoint: &ModelEndpointKey) {
        for cell in self.inner.sessions.cells() {
            if let Err(error) = self.take_listed_context_window(&cell, endpoint).await {
                log::debug!("a session did not take its listed context window: {error}");
            }
        }
    }

    async fn take_listed_context_window(
        &self,
        cell: &Arc<SessionCell>,
        endpoint: &ModelEndpointKey,
    ) -> Result<(), AgentError> {
        let staged = {
            let mut guard = cell.lock()?;
            let record = &mut *guard;
            let listed = record
                .settings
                .model
                .route_key(&record.settings.config.model)
                .is_some_and(|route| route.endpoint() == endpoint);
            if !listed || ensure_no_staged_commit(record).is_err() {
                return Ok(());
            }
            let rollback = record.rollback_state();
            if self.refresh_context_window(record).is_none() {
                return Ok(());
            }
            // Not `updated_at`: a listing is not activity, and the session
            // list is ordered by it.
            record.status.revision = record
                .status
                .revision
                .checked_add(1)
                .expect("agent snapshot revision exhausted");
            StagedPersistence::commit(cell, record, rollback)
        };
        self.persist_staged(staged).await
    }

    /// Gives `record`'s session the window its route resolves to now, and
    /// measures its usage against it too, since recovery refuses usage
    /// measured against another window. Called under the session's lock,
    /// in a commit: the one that admits a turn, records a response, takes a
    /// listing, or follows a refusal that taught a window. A configured window wins; a window no longer known
    /// is kept, unless it was recorded as configured, learned or from
    /// models.dev (the configuration, the learned fact or the copy of
    /// models.dev is gone), and then the session's window is unknown.
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
            // Only a window recorded as configured, learned or from
            // models.dev: one without a source (written before sources were
            // kept) is kept, and so is a reported one.
            None if matches!(
                config.context_window_source,
                Some(
                    ModelMetadataSource::Configured
                        | ModelMetadataSource::Learned
                        | ModelMetadataSource::Catalog
                )
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
