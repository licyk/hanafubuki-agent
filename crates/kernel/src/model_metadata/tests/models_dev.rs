//! models.dev's catalogue in the registry: behind what the person
//! configures, what the Provider's refusals teach and what its listing
//! reports, before the built-in table; the Provider's own entry first; and
//! its copy kept for a week, apart from the learned facts.

use super::*;

use hanafubuki_agent_model_api::{ModelEndpoint, ModelProviderNames, ModelsDevCatalog};
use serde_json::json;

use super::super::models_dev::MODELS_DEV_TTL_MS;

const PROVIDER: &str = "https://api.provider.test/v1";

/// A catalogue in models.dev's own form: `gpt-5` at two providers, the
/// smaller window at the other one.
fn catalog() -> ModelsDevCatalog {
    let document = json!({
        "provider": {"models": {
            "gpt-5": {
                "reasoning": true,
                "modalities": {"input": ["text", "image"]},
                "limit": {"context": 400_000, "output": 128_000}
            },
            "text-only": {
                "reasoning": false,
                "modalities": {"input": ["text"]},
                "limit": {"context": 32_000}
            }
        }},
        "reseller": {"models": {
            "openai/gpt-5": {"limit": {"context": 128_000, "output": 32_000}}
        }}
    });
    ModelsDevCatalog::from_api_json(document.to_string().as_bytes()).unwrap()
}

fn opened() -> (ModelMetadataRegistry, Arc<RecordingStore>) {
    let store = Arc::new(RecordingStore::default());
    (ModelMetadataRegistry::open(store.clone(), NOW), store)
}

fn named(registry: &ModelMetadataRegistry, provider_id: &str) {
    registry.name_provider(
        endpoint(PROVIDER),
        ModelProviderNames::new(provider_id, &ModelEndpoint::parse(PROVIDER).unwrap()),
    );
}

fn resolved(registry: &ModelMetadataRegistry, route: &ModelRouteKey, now_ms: u64) -> ModelMetadata {
    registry.resolve(
        Some(route),
        route.model(),
        ConfiguredModelFacts::default(),
        now_ms,
    )
}

#[test]
fn models_dev_fills_only_what_nothing_better_says() {
    let (registry, _) = opened();
    registry.record_models_dev(catalog(), NOW);
    named(&registry, "provider");
    let gpt = route(PROVIDER, "gpt-5");

    let alone = resolved(&registry, &gpt, NOW);
    assert_eq!(
        alone.context_window,
        Some(sourced(400_000, ModelMetadataSource::Catalog, NOW)),
        "models.dev beats the built-in window (272000)"
    );
    assert_eq!(
        alone.max_output_tokens,
        Some(sourced(128_000, ModelMetadataSource::Catalog, NOW))
    );
    assert_eq!(
        alone.reasoning,
        Some(sourced(
            ModelReasoningSupport::Supported {
                efforts: Vec::new(),
                default: None,
            },
            ModelMetadataSource::Catalog,
            NOW
        ))
    );
    assert_eq!(
        registry.observed_image_input(&gpt, None, NOW),
        ModelImageInputCapability::Supported
    );

    let mut listed = entry("gpt-5", Some(300_000));
    listed.image_input = Some(false);
    registry.record_listing(&endpoint(PROVIDER), &[listed], NOW);
    registry.learn_reasoning_unsupported(&gpt, NOW);
    let behind = resolved(&registry, &gpt, NOW);
    assert_eq!(
        behind.context_window,
        Some(sourced(300_000, ModelMetadataSource::Reported, NOW)),
        "a reported window beats models.dev"
    );
    assert_eq!(
        behind.image_input,
        Some(sourced(false, ModelMetadataSource::Reported, NOW)),
        "models.dev's yes never contradicts a reported no"
    );
    assert_eq!(
        behind.reasoning.map(|fact| fact.source),
        Some(ModelMetadataSource::Learned),
        "nor a learned one"
    );
    assert_eq!(
        behind.max_output_tokens.map(|fact| fact.source),
        Some(ModelMetadataSource::Catalog),
        "what the listing leaves unknown is still filled"
    );
    let configured = registry.resolve(
        Some(&gpt),
        "gpt-5",
        ConfiguredModelFacts {
            max_output_tokens: Some(8_000),
            ..ConfiguredModelFacts::default()
        },
        NOW,
    );
    assert_eq!(
        configured.max_output_tokens.map(|fact| fact.source),
        Some(ModelMetadataSource::Configured)
    );

    // A learned window under models.dev's is taken; models.dev's "no" to
    // images is not kept.
    let text_only = route(PROVIDER, "text-only");
    registry.learn_context_window(&text_only, 16_000, NOW);
    let learned = resolved(&registry, &text_only, NOW);
    assert_eq!(
        learned.context_window,
        Some(sourced(16_000, ModelMetadataSource::Learned, NOW))
    );
    assert_eq!(
        registry.observed_image_input(&text_only, None, NOW),
        ModelImageInputCapability::Unknown
    );
}

#[test]
fn the_providers_own_entry_comes_first_else_the_smallest() {
    let (registry, _) = opened();
    registry.record_models_dev(catalog(), NOW);
    let gpt = route(PROVIDER, "gpt-5");
    let window = |registry: &ModelMetadataRegistry| {
        resolved(registry, &gpt, NOW)
            .context_window
            .map(|fact| fact.value)
    };

    assert_eq!(window(&registry), Some(128_000), "not named: the smallest");
    named(&registry, "reseller");
    assert_eq!(window(&registry), Some(128_000));
    named(&registry, "provider");
    assert_eq!(window(&registry), Some(400_000), "the host names it too");
    // A session not bound to its Provider has no route, and no name.
    assert_eq!(
        registry
            .resolve(None, "GPT-5-latest", ConfiguredModelFacts::default(), NOW)
            .context_window
            .map(|fact| fact.value),
        Some(128_000)
    );
}

#[test]
fn the_copy_is_kept_for_a_week_and_saved_apart_from_the_learned_facts() {
    let (registry, store) = opened();
    let gpt = route(PROVIDER, "gpt-5");
    registry.record_models_dev(catalog(), NOW);
    registry.learn_context_window(&gpt, 50_000, NOW);
    assert_eq!(registry.models_dev_fetched_at_ms(NOW + 1), Some(NOW));

    assert_eq!(registry.forget_learned(&endpoint(PROVIDER), None, NOW), 1);
    let saved = store.saved.lock().last().cloned().unwrap();
    let (reopened, _) = {
        let store = Arc::new(RecordingStore {
            loaded: serde_json::from_value(serde_json::to_value(saved).unwrap()).unwrap(),
            saved: Mutex::default(),
        });
        (ModelMetadataRegistry::open(store.clone(), NOW + 1), store)
    };
    assert_eq!(
        resolved(&reopened, &gpt, NOW + 1)
            .context_window
            .map(|fact| fact.source),
        Some(ModelMetadataSource::Catalog),
        "forgetting what was learned keeps models.dev"
    );

    let expired = NOW + MODELS_DEV_TTL_MS + 1;
    assert_eq!(reopened.models_dev_fetched_at_ms(expired), None);
    assert_eq!(
        resolved(&reopened, &gpt, expired)
            .context_window
            .map(|fact| fact.source),
        Some(ModelMetadataSource::BuiltIn)
    );
}

#[test]
fn a_copy_that_does_not_read_is_none_and_the_rest_is_kept() {
    let model = route(PROVIDER, "model");
    let (registry, store) = opened();
    registry.record_listing(&endpoint(PROVIDER), &[entry("model", Some(64_000))], NOW);
    let mut saved = serde_json::to_value(store.saved.lock().last().cloned().unwrap()).unwrap();
    saved["modelsDev"] = json!({"fetchedAtMs": "yesterday", "catalog": []});

    let snapshot: ModelMetadataSnapshot = serde_json::from_value(saved).unwrap();
    assert!(snapshot.models_dev.is_none());
    let reopened = ModelMetadataRegistry::open(
        Arc::new(RecordingStore {
            loaded: snapshot,
            saved: Mutex::default(),
        }),
        NOW,
    );
    assert_eq!(window(&reopened, None, &model, NOW), Some(64_000));
}

#[test]
fn forgetting_the_copy_saves_once() {
    let (registry, store) = opened();
    assert!(!registry.forget_models_dev());
    assert!(store.saved.lock().is_empty());
    registry.record_models_dev(catalog(), NOW);
    assert!(registry.forget_models_dev());
    assert_eq!(store.saved.lock().len(), 2);
    assert!(store.saved.lock()[1].models_dev.is_none());
    assert_eq!(registry.models_dev_fetched_at_ms(NOW), None);
}
