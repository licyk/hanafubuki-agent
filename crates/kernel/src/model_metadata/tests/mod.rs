use std::sync::Arc;

use hanafubuki_agent_model_api::{
    ModelApiProtocol, ModelCatalogEntry, ModelEndpoint, ModelEndpointKey,
    ModelImageInputCapability, ModelMetadataSource, ModelReasoningEffort, ModelReasoningSupport,
    ModelRouteKey, Sourced,
};
use parking_lot::Mutex;

use super::state::{LEARNED_TTL_MS, MAX_ROUTES, REPORTED_TTL_MS};
use super::*;

mod forget;
mod listing;
mod models_dev;
mod persistence;
mod precedence;

const NOW: u64 = 1_000_000_000;

fn endpoint(base_url: &str) -> ModelEndpointKey {
    ModelEndpointKey::new(
        ModelApiProtocol::Responses,
        &ModelEndpoint::parse(base_url).unwrap(),
    )
}

fn route(base_url: &str, model: &str) -> ModelRouteKey {
    endpoint(base_url).route(model)
}

fn entry(id: &str, context_window: Option<u64>) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: id.to_string(),
        display_name: None,
        default_reasoning_effort: None,
        supported_reasoning_efforts: Vec::new(),
        context_window,
        max_output_tokens: None,
        image_input: None,
        reasoning: None,
    }
}

/// A store that keeps every snapshot it is given, and loads `loaded`.
#[derive(Default)]
struct RecordingStore {
    loaded: ModelMetadataSnapshot,
    saved: Mutex<Vec<ModelMetadataSnapshot>>,
}

impl ModelMetadataStore for RecordingStore {
    fn load(&self) -> ModelMetadataSnapshot {
        self.loaded.clone()
    }

    fn save(&self, snapshot: ModelMetadataSnapshot) {
        self.saved.lock().push(snapshot);
    }
}

fn registry() -> ModelMetadataRegistry {
    ModelMetadataRegistry::open(Arc::new(NoopModelMetadataStore), NOW)
}

fn window(
    registry: &ModelMetadataRegistry,
    configured: Option<u64>,
    route: &ModelRouteKey,
    now_ms: u64,
) -> Option<u64> {
    registry
        .context_window(configured, Some(route), route.model(), now_ms)
        .map(|fact| fact.value)
}

fn sourced<T>(value: T, source: ModelMetadataSource, observed_at_ms: u64) -> Sourced<T> {
    Sourced {
        value,
        source,
        observed_at_ms,
    }
}
