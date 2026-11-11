//! Forgetting what the Provider's refusals taught, and what the person
//! configures over every other source.

use super::*;

const PROVIDER: &str = "https://provider.test/v1";
const OTHER: &str = "https://other.test/v1";

fn opened() -> (ModelMetadataRegistry, Arc<RecordingStore>) {
    let store = Arc::new(RecordingStore::default());
    (ModelMetadataRegistry::open(store.clone(), NOW), store)
}

fn teach(registry: &ModelMetadataRegistry, route: &ModelRouteKey) {
    registry.learn_context_window(route, 32_000, NOW);
    registry.learn_image_input_unsupported(route, NOW);
    registry.learn_reasoning_unsupported(route, NOW);
}

fn learned_anything(registry: &ModelMetadataRegistry, route: &ModelRouteKey) -> bool {
    let metadata = registry.resolve(
        Some(route),
        route.model(),
        ConfiguredModelFacts::default(),
        NOW + 1,
    );
    [
        metadata.context_window.map(|fact| fact.source),
        metadata.image_input.map(|fact| fact.source),
        metadata.reasoning.map(|fact| fact.source),
    ]
    .contains(&Some(ModelMetadataSource::Learned))
}

#[test]
fn a_model_forgets_what_it_was_taught_and_keeps_what_was_reported() {
    let (registry, store) = opened();
    let model = route(PROVIDER, "model");
    let sibling = route(PROVIDER, "sibling");
    registry.record_listing(
        &endpoint(PROVIDER),
        &[entry("model", Some(128_000)), entry("sibling", None)],
        NOW,
    );
    teach(&registry, &model);
    teach(&registry, &sibling);
    let saves = store.saved.lock().len();

    assert_eq!(
        registry.forget_learned(&endpoint(PROVIDER), Some("model"), NOW + 1),
        1
    );

    assert!(!learned_anything(&registry, &model));
    assert_eq!(window(&registry, None, &model, NOW + 1), Some(128_000));
    assert_eq!(
        registry.observed_image_input(&model, None, NOW + 1),
        ModelImageInputCapability::Unknown
    );
    assert!(
        learned_anything(&registry, &sibling),
        "only the model named"
    );
    assert_eq!(store.saved.lock().len(), saves + 1, "saved once");
    let saved = store.saved.lock().last().cloned().unwrap();
    let reopened = ModelMetadataRegistry::open(
        Arc::new(RecordingStore {
            loaded: saved,
            saved: Mutex::default(),
        }),
        NOW + 1,
    );
    assert!(!learned_anything(&reopened, &model), "the next run too");
}

#[test]
fn an_endpoint_forgets_every_model_and_nothing_elsewhere() {
    let (registry, store) = opened();
    let first = route(PROVIDER, "first");
    let second = route(PROVIDER, "second");
    let elsewhere = route(OTHER, "first");
    for route in [&first, &second, &elsewhere] {
        teach(&registry, route);
    }

    assert_eq!(registry.forget_learned(&endpoint(PROVIDER), None, NOW), 2);

    assert!(!learned_anything(&registry, &first));
    assert!(!learned_anything(&registry, &second));
    assert!(learned_anything(&registry, &elsewhere));
    let saves = store.saved.lock().len();
    assert_eq!(registry.forget_learned(&endpoint(PROVIDER), None, NOW), 0);
    assert_eq!(
        store.saved.lock().len(),
        saves,
        "nothing to forget, no save"
    );
}

#[test]
fn configured_facts_beat_learned_and_reported_ones() {
    let registry = registry();
    let model = route(PROVIDER, "model");
    registry.record_listing(
        &endpoint(PROVIDER),
        &[ModelCatalogEntry {
            max_output_tokens: Some(64_000),
            reasoning: Some(true),
            supported_reasoning_efforts: vec![ModelReasoningEffort::Low],
            ..entry("model", Some(128_000))
        }],
        NOW,
    );
    teach(&registry, &model);
    let configured = |reasoning| ConfiguredModelFacts {
        context_window: Some(200_000),
        max_output_tokens: Some(8_000),
        image_input: Some(true),
        reasoning,
    };

    let resolved = registry.resolve(
        Some(&model),
        "model",
        configured(Some(ModelReasoningSupport::Supported {
            efforts: vec![ModelReasoningEffort::High],
            default: None,
        })),
        NOW,
    );

    let configured_source = ModelMetadataSource::Configured;
    assert_eq!(
        resolved.context_window,
        Some(sourced(200_000, configured_source, 0)),
        "larger than the learned window, and still taken"
    );
    assert_eq!(
        resolved.max_output_tokens,
        Some(sourced(8_000, configured_source, 0))
    );
    assert_eq!(
        resolved.image_input,
        Some(sourced(true, configured_source, 0))
    );
    assert_eq!(
        resolved.reasoning,
        Some(sourced(
            ModelReasoningSupport::Supported {
                efforts: vec![ModelReasoningEffort::High],
                default: None,
            },
            configured_source,
            0
        ))
    );
    let none = registry.resolve(
        Some(&model),
        "model",
        configured(Some(ModelReasoningSupport::Unsupported)),
        NOW,
    );
    assert_eq!(
        none.reasoning.map(|fact| fact.value),
        Some(ModelReasoningSupport::Unsupported)
    );
}
