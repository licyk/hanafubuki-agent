//! What the person configured for a session's model, and what a host reads
//! and clears of the model metadata: a model's resolved facts, a session's,
//! and forgetting what the Provider's refusals taught.

use hanafubuki_agent_model_api::{
    ModelEndpoint, ModelEndpointKey, ModelImageInputPolicy, ModelMetadata, ModelRouteKey, Sourced,
};

use super::*;
use crate::model_metadata::ConfiguredModelFacts;
use crate::AgentConfiguredModelFacts;

impl AgentService {
    /// What is known of `model` at the Provider at `base_url`, with what
    /// the person `configured` for it, each fact with its source: none
    /// when `base_url` is not an endpoint.
    pub fn model_metadata(
        &self,
        protocol: ModelApiProtocol,
        base_url: &str,
        model: &str,
        configured: &AgentConfiguredModelFacts,
    ) -> Option<ModelMetadata> {
        let route = ModelRouteKey::new(protocol, &ModelEndpoint::parse(base_url).ok()?, model);
        Some(self.inner.model_metadata.resolve(
            Some(&route),
            model,
            ConfiguredModelFacts::from_configured(configured),
            now_millis_u64(),
        ))
    }

    /// What the session `session_id` knows of its model: its route's facts
    /// with what the person configured (none but those while it is not
    /// bound to its Provider), and the window the session has, with its
    /// source.
    pub fn session_model_metadata(
        &self,
        session_id: AgentSessionId,
    ) -> Result<ModelMetadata, AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock()?;
        let config = &record.settings.config;
        let route = record.settings.model.route_key(&config.model);
        let mut metadata = self.inner.model_metadata.resolve(
            route.as_ref(),
            &config.model,
            configured_facts(config),
            now_millis_u64(),
        );
        metadata.context_window = config
            .context_window_tokens
            .zip(config.context_window_origin())
            .map(|(value, source)| Sourced {
                value,
                source,
                observed_at_ms: 0,
            });
        Ok(metadata)
    }

    /// Forgets what the Provider at `base_url` taught by its refusals of
    /// `model`, or of every model there when none is named: learned
    /// windows, and a learned "no" to images or to reasoning. A session
    /// takes it at its next turn. Returns how many models had something to
    /// forget.
    pub fn forget_learned_model_facts(
        &self,
        protocol: ModelApiProtocol,
        base_url: &str,
        model: Option<&str>,
    ) -> Result<usize, AgentError> {
        let endpoint = ModelEndpoint::parse(base_url)
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        Ok(self.inner.model_metadata.forget_learned(
            &ModelEndpointKey::new(protocol, &endpoint),
            model,
            now_millis_u64(),
        ))
    }
}

/// What the person configured for the session's model, under its input
/// policy.
pub(super) fn configured_facts(config: &AgentSessionConfig) -> ConfiguredModelFacts {
    ConfiguredModelFacts {
        image_input: configured_image_input(config),
        ..ConfiguredModelFacts::from_configured(&config.configured_facts)
    }
}

/// The image input the person configured for the session's model, which
/// beats every other source: `"disabled"` is a "no" whatever the model's
/// own setting says (`effective` applies it too), else the model's.
pub(super) fn configured_image_input(config: &AgentSessionConfig) -> Option<bool> {
    if config.input_capabilities.image_input == ModelImageInputPolicy::Disabled {
        return Some(false);
    }
    config.configured_facts.image_input
}
