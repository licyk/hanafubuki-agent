//! What is known about each model the Agent reaches (its route: protocol,
//! endpoint and model id), fact by fact: its context window, largest
//! output, reasoning and image input. A fact comes from the person's
//! configuration, from a Provider's refusal (learned), from its listing
//! (reported), from a third-party catalog (none yet) or from the built-in
//! window table, and is resolved per fact in that order (`resolve`). The
//! facts outlive the run through the [`ModelMetadataStore`] the kernel's
//! owner injects.

use std::sync::Arc;

use hanafubuki_agent_model_api::{
    ModelCatalogEntry, ModelEndpointKey, ModelImageInputCapability, ModelMetadata,
    ModelMetadataSource, ModelRouteKey, Sourced,
};
use parking_lot::Mutex;

mod built_in;
mod resolve;
mod state;
mod store;

use state::RegistryState;
pub(crate) use store::NoopModelMetadataStore;
pub use store::{ModelMetadataSnapshot, ModelMetadataStore};

/// What the person configured for a route; each fact set wins over every
/// other source.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ConfiguredModelFacts {
    pub(crate) context_window: Option<u64>,
    pub(crate) image_input: Option<bool>,
}

/// The facts per route, for the whole kernel. Every method takes the time,
/// in milliseconds since the Unix epoch, that its facts expire against.
pub(crate) struct ModelMetadataRegistry {
    state: Mutex<RegistryState>,
    store: Arc<dyn ModelMetadataStore>,
}

impl ModelMetadataRegistry {
    /// The registry over what `store` kept.
    pub(crate) fn open(store: Arc<dyn ModelMetadataStore>, now_ms: u64) -> Self {
        let state = RegistryState::restore(store.load(), now_ms);
        Self {
            state: Mutex::new(state),
            store,
        }
    }

    /// Records `entries`, the listing of `endpoint`, model by model; see
    /// `RegistryState::record_listing`.
    pub(crate) fn record_listing(
        &self,
        endpoint: &ModelEndpointKey,
        entries: &[ModelCatalogEntry],
        now_ms: u64,
    ) {
        self.change(|state| state.record_listing(endpoint, entries, now_ms));
    }

    /// Learns that `route` refuses images.
    pub(crate) fn learn_image_input_unsupported(&self, route: &ModelRouteKey, now_ms: u64) {
        self.change(|state| {
            state.learn(route, now_ms, |learned| {
                learned.image_input = Some(Sourced {
                    value: false,
                    source: ModelMetadataSource::Learned,
                    observed_at_ms: now_ms,
                });
            })
        });
    }

    /// Learns that `route`'s context window is at most `tokens`, from a
    /// refusal: kept while it is the smallest the route's live refusals
    /// taught (an expired one no longer counts).
    pub(crate) fn learn_context_window(&self, route: &ModelRouteKey, tokens: u64, now_ms: u64) {
        self.change(|state| {
            let smaller_known = state
                .live_facts(route, now_ms)
                .and_then(|facts| facts.learned.context_window)
                .is_some_and(|known| known.value <= tokens);
            !smaller_known
                && state.learn(route, now_ms, |learned| {
                    learned.context_window = Some(Sourced {
                        value: tokens,
                        source: ModelMetadataSource::Learned,
                        observed_at_ms: now_ms,
                    });
                })
        });
    }

    /// What is known about `model`, at `route` when the transport names
    /// one, with what the person `configured`.
    pub(crate) fn resolve(
        &self,
        route: Option<&ModelRouteKey>,
        model: &str,
        configured: ConfiguredModelFacts,
        now_ms: u64,
    ) -> ModelMetadata {
        let facts = route.and_then(|route| self.state.lock().live_facts(route, now_ms));
        resolve::resolve(facts.as_ref(), configured, built_in::context_window(model))
    }

    /// The context window a session on `model` is given, and its source.
    pub(crate) fn context_window(
        &self,
        configured: Option<u64>,
        route: Option<&ModelRouteKey>,
        model: &str,
        now_ms: u64,
    ) -> Option<Sourced<u64>> {
        let configured = ConfiguredModelFacts {
            context_window: configured,
            ..ConfiguredModelFacts::default()
        };
        self.resolve(route, model, configured, now_ms)
            .context_window
    }

    /// The largest output `model` at `route` is known to give.
    pub(crate) fn max_output_tokens(
        &self,
        route: &ModelRouteKey,
        model: &str,
        now_ms: u64,
    ) -> Option<u64> {
        self.resolve(Some(route), model, ConfiguredModelFacts::default(), now_ms)
            .max_output_tokens
            .map(|fact| fact.value)
    }

    /// When `endpoint`'s latest listing was recorded, in milliseconds since
    /// the Unix epoch; none when it never was (or no longer is kept).
    pub(crate) fn listed_at_ms(&self, endpoint: &ModelEndpointKey) -> Option<u64> {
        self.state
            .lock()
            .listings
            .get(endpoint)
            .map(|listing| listing.listed_at_ms)
    }

    /// What a session observes of `route`'s image input: its resolved fact
    /// (`resolve`, with what the person `configured`), supported or
    /// unsupported, else unknown.
    pub(crate) fn observed_image_input(
        &self,
        route: &ModelRouteKey,
        configured: Option<bool>,
        now_ms: u64,
    ) -> ModelImageInputCapability {
        let configured = ConfiguredModelFacts {
            image_input: configured,
            ..ConfiguredModelFacts::default()
        };
        match self
            .resolve(Some(route), route.model(), configured, now_ms)
            .image_input
        {
            Some(fact) if fact.value => ModelImageInputCapability::Supported,
            Some(_) => ModelImageInputCapability::Unsupported,
            None => ModelImageInputCapability::Unknown,
        }
    }

    /// Applies `change`, and saves the state when it changed. The snapshot
    /// is handed to the store under the lock, so the store receives them
    /// in the order of the changes.
    fn change(&self, change: impl FnOnce(&mut RegistryState) -> bool) {
        let mut state = self.state.lock();
        if change(&mut state) {
            self.store.save(state.snapshot());
        }
    }
}

#[cfg(test)]
mod tests;
