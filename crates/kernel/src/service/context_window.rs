//! A session's context window, resolved again as each turn starts: the
//! model metadata may know its model's window better than when the session
//! was created (a listing that landed later), and the session takes it.

use hanafubuki_agent_model_api::{ModelMetadataSource, Sourced};

use super::{now_millis_u64, AgentService, AgentSessionConfig, SessionRecord};

impl AgentService {
    /// Gives `record`'s session the window its route resolves to now, and
    /// measures its usage against it too, since recovery refuses usage
    /// measured against another window. Called under the session's lock,
    /// in the commit that admits a turn. A configured window is never
    /// replaced, and a window no longer known keeps the last one.
    pub(super) fn refresh_context_window(&self, record: &mut SessionRecord) {
        let config = &record.settings.config;
        if config.context_window_origin() == Some(ModelMetadataSource::Configured) {
            return;
        }
        // A session not bound to its Provider yet (recovered, its transport
        // unavailable) has no route: the built-in table alone would take
        // the place of what its Provider reported.
        let Some(route) = record.settings.model.route_key(&config.model) else {
            return;
        };
        let resolved = self.inner.model_metadata.context_window(
            None,
            Some(&route),
            &config.model,
            now_millis_u64(),
        );
        let Some(window) = changed_window(config, resolved) else {
            return;
        };
        if let Some(usage) = record.runtime.usage.as_mut() {
            usage.context_window_tokens = Some(window.value);
        }
        record.settings.config.set_context_window(Some(window));
    }
}

/// `resolved` when it differs from `config`'s window or its source; none
/// when it is unknown or the same.
fn changed_window(
    config: &AgentSessionConfig,
    resolved: Option<Sourced<u64>>,
) -> Option<Sourced<u64>> {
    resolved.filter(|window| {
        config.context_window_tokens != Some(window.value)
            || config.context_window_source != Some(window.source)
    })
}

#[cfg(test)]
mod tests;
