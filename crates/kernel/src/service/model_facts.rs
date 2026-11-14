//! What the person configured for a session's model, and what a host reads
//! and clears of the model metadata: a model's resolved facts, a listing's,
//! a session's, forgetting what the Provider's refusals taught, and the
//! copy of models.dev's catalogue the host fetches. Also the model session a
//! structured request samples, which answers and learns its routes'
//! structured-output rung from the model metadata.

use hanafubuki_agent_model_api::{
    ModelCompactionResult, ModelEndpoint, ModelEndpointKey, ModelFuture, ModelImageInputPolicy,
    ModelMetadata, ModelProviderNames, ModelResult, ModelRouteKey, ModelStream,
    ModelStructuredOutputCapability, ModelsDevCatalog, Sourced, ToolProviderCapabilities,
};

use super::*;
use crate::model_metadata::{ConfiguredModelFacts, ModelMetadataRegistry};
use crate::AgentConfiguredModelFacts;

impl AgentService {
    /// What is known of `model` at the Provider `provider_id` at
    /// `base_url`, with what the person `configured` for it, each fact with
    /// its source: none when `base_url` is not an endpoint.
    pub fn model_metadata(
        &self,
        provider_id: &str,
        protocol: ModelApiProtocol,
        base_url: &str,
        model: &str,
        configured: &AgentConfiguredModelFacts,
    ) -> Option<ModelMetadata> {
        let endpoint = ModelEndpoint::parse(base_url).ok()?;
        self.name_model_provider(provider_id, protocol, &endpoint);
        let route = ModelRouteKey::new(protocol, &endpoint, model);
        Some(self.inner.model_metadata.resolve(
            Some(&route),
            model,
            ConfiguredModelFacts::from_configured(configured),
            now_millis_u64(),
        ))
    }

    /// What is known of each of `entries`, the listing just read of the
    /// Provider `provider_id` at `base_url`, in their order: what
    /// [`Self::model_metadata`] gives each model, with what the person
    /// `configured` for it, but with its own entry as what the listing
    /// reported, whatever was recorded of it. None known of any when
    /// `base_url` is not an endpoint.
    pub fn resolve_listing(
        &self,
        provider_id: &str,
        protocol: ModelApiProtocol,
        base_url: &str,
        entries: &[ModelCatalogEntry],
        configured: impl Fn(&str) -> AgentConfiguredModelFacts,
    ) -> Vec<ModelMetadata> {
        let Ok(endpoint) = ModelEndpoint::parse(base_url) else {
            return vec![ModelMetadata::default(); entries.len()];
        };
        self.name_model_provider(provider_id, protocol, &endpoint);
        self.inner.model_metadata.resolve_listing(
            &ModelEndpointKey::new(protocol, &endpoint),
            entries,
            |model| ConfiguredModelFacts::from_configured(&configured(model)),
            now_millis_u64(),
        )
    }

    /// What the session `session_id` knows of its model: its route's facts
    /// with what the person configured (none but those while it is not
    /// bound to its Provider), and the window the session has, with its
    /// source; the resolved window while the session has none.
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
        if let Some((value, source)) = config
            .context_window_tokens
            .zip(config.context_window_origin())
        {
            metadata.context_window = Some(Sourced {
                value,
                source,
                observed_at_ms: 0,
            });
        }
        Ok(metadata)
    }

    /// Forgets what the Provider at `base_url` taught by its refusals of
    /// `model`, or of every model there when none is named: learned
    /// windows, a learned "no" to images or to reasoning, and a learned
    /// structured-output rung. A session takes it at its next turn or
    /// response. Returns how many models had something to forget.
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

    /// Keeps `catalog`, models.dev's catalogue just fetched, in place of
    /// the copy before: what it says of a model fills what neither the
    /// person, the Provider's refusals nor its listing say, for a week
    /// unless a newer copy replaces it.
    pub fn record_models_dev_catalog(&self, catalog: ModelsDevCatalog) {
        self.inner
            .model_metadata
            .record_models_dev(catalog, now_millis_u64());
    }

    /// Drops the copy of models.dev's catalogue (the person turned it off);
    /// returns whether there was one.
    pub fn forget_models_dev_catalog(&self) -> bool {
        self.inner.model_metadata.forget_models_dev()
    }

    /// When the copy of models.dev's catalogue still used was fetched, in
    /// milliseconds since the Unix epoch; none without one.
    pub fn models_dev_fetched_at(&self) -> Option<u64> {
        self.inner
            .model_metadata
            .models_dev_fetched_at_ms(now_millis_u64())
    }

    /// Tells the model metadata that the Provider at `endpoint` over
    /// `protocol` is `provider_id`, which picks its own entries in
    /// models.dev.
    pub(super) fn name_model_provider(
        &self,
        provider_id: &str,
        protocol: ModelApiProtocol,
        endpoint: &ModelEndpoint,
    ) {
        self.inner.model_metadata.name_provider(
            ModelEndpointKey::new(protocol, endpoint),
            ModelProviderNames::new(provider_id, endpoint),
        );
    }

    /// `session`, answering the structured-output rung of each route from
    /// the model metadata and teaching it what the route refused.
    pub(super) fn learning_model_session(
        &self,
        session: Arc<dyn ModelSession>,
    ) -> Arc<dyn ModelSession> {
        Arc::new(LearningModelSession {
            session,
            metadata: self.inner.model_metadata.clone(),
        })
    }

    /// `name_model_provider` from a base URL, which may not be an
    /// endpoint (then nothing is named).
    pub(super) fn name_model_provider_at(
        &self,
        provider_id: &str,
        protocol: ModelApiProtocol,
        base_url: &str,
    ) {
        if let Ok(endpoint) = ModelEndpoint::parse(base_url) {
            self.name_model_provider(provider_id, protocol, &endpoint);
        }
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

/// A model session whose routes' structured-output rung is the protocol's,
/// lowered by what their refusals taught. It holds the registry, not the
/// service, so a session kept by its owner keeps no service alive.
struct LearningModelSession {
    session: Arc<dyn ModelSession>,
    metadata: Arc<ModelMetadataRegistry>,
}

impl ModelSession for LearningModelSession {
    fn compaction_capability(&self) -> hanafubuki_agent_model_api::ModelCompactionCapability {
        self.session.compaction_capability()
    }

    fn tool_capabilities(&self) -> ToolProviderCapabilities {
        self.session.tool_capabilities()
    }

    fn structured_output_capability(&self) -> ModelStructuredOutputCapability {
        self.session.structured_output_capability()
    }

    fn structured_output_capability_for(&self, model: &str) -> ModelStructuredOutputCapability {
        let protocol = self.session.structured_output_capability_for(model);
        self.session
            .route_key(model)
            .and_then(|route| self.metadata.structured_output(&route, now_millis_u64()))
            .map_or(protocol, |learned| protocol.max(learned))
    }

    fn learn_structured_output(&self, model: &str, rung: ModelStructuredOutputCapability) {
        if let Some(route) = self.session.route_key(model) {
            self.metadata
                .learn_structured_output(&route, rung, now_millis_u64());
        }
    }

    fn route_key(&self, model: &str) -> Option<ModelRouteKey> {
        self.session.route_key(model)
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.session.stream(request, cancellation)
    }

    fn compact(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelCompactionResult>> {
        self.session.compact(request, cancellation)
    }
}
