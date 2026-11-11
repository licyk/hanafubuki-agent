use super::*;

use serde_json::json;

const PROVIDER: &str = "https://provider.test/v1";

fn opened(loaded: ModelMetadataSnapshot) -> (ModelMetadataRegistry, Arc<RecordingStore>) {
    let store = Arc::new(RecordingStore {
        loaded,
        saved: Mutex::default(),
    });
    (ModelMetadataRegistry::open(store.clone(), NOW), store)
}

fn last_saved(store: &RecordingStore) -> ModelMetadataSnapshot {
    store
        .saved
        .lock()
        .last()
        .cloned()
        .expect("a snapshot was saved")
}

#[test]
fn only_a_change_is_saved() {
    let (registry, store) = opened(ModelMetadataSnapshot::default());
    let model = route(PROVIDER, "model");
    window(&registry, None, &model, NOW);
    registry.observed_image_input(&model, None, NOW);
    assert!(store.saved.lock().is_empty());

    registry.record_listing(&endpoint(PROVIDER), &[entry("model", Some(64_000))], NOW);
    registry.learn_image_input_unsupported(&model, NOW);

    assert_eq!(store.saved.lock().len(), 2);
}

#[test]
fn what_was_saved_is_what_the_next_run_knows() {
    let (registry, store) = opened(ModelMetadataSnapshot::default());
    let listed = route(PROVIDER, "listed");
    let refused = route(PROVIDER, "refused");
    registry.record_listing(&endpoint(PROVIDER), &[entry("listed", Some(64_000))], NOW);
    registry.learn_image_input_unsupported(&refused, NOW);
    let saved = serde_json::to_value(last_saved(&store)).unwrap();

    let (reopened, store) = opened(serde_json::from_value(saved).unwrap());

    assert_eq!(window(&reopened, None, &listed, NOW + 1), Some(64_000));
    assert_eq!(
        reopened.observed_image_input(&refused, None, NOW + 1),
        ModelImageInputCapability::Unsupported
    );
    assert!(store.saved.lock().is_empty(), "opening saves nothing");
}

#[test]
fn a_held_listing_is_remembered_by_the_next_run() {
    let (registry, store) = opened(ModelMetadataSnapshot::default());
    let provider = endpoint(PROVIDER);
    registry.record_listing(
        &provider,
        &[
            entry("a", Some(100_000)),
            entry("b", None),
            entry("c", None),
        ],
        NOW,
    );
    registry.record_listing(&provider, &[entry("a", Some(50_000))], NOW + 1);

    let (reopened, _) = opened(last_saved(&store));
    reopened.record_listing(&provider, &[entry("a", Some(50_000))], NOW + 2);

    assert_eq!(
        window(&reopened, None, &route(PROVIDER, "a"), NOW + 2),
        Some(50_000)
    );
}

#[test]
fn a_saved_snapshot_reads_tolerantly() {
    let model = route(PROVIDER, "model");
    let other = route(PROVIDER, "other");
    let snapshot: ModelMetadataSnapshot = serde_json::from_value(json!({
        "unknown": true,
        "routes": [
            {
                "route": model,
                "unknownField": 1,
                "reported": {
                    "contextWindow": {"value": 64_000, "source": "configured", "observedAtMs": NOW},
                    "maxOutputTokens": "many",
                    "imageInput": {"value": true, "source": "reported", "observedAtMs": NOW * 2}
                },
                "learned": "not facts"
            },
            {"route": {"model": "no endpoint"}},
            "not a route",
            {
                "route": other,
                "learned": {"imageInput": {"value": false, "source": "learned", "observedAtMs": NOW}}
            }
        ],
        "listings": {"not": "a list"}
    }))
    .unwrap();

    let (registry, _) = opened(snapshot);

    let resolved = registry.resolve(Some(&model), "model", Default::default(), NOW);
    assert_eq!(
        resolved.context_window,
        Some(sourced(64_000, ModelMetadataSource::Reported, NOW)),
        "a saved fact keeps its bucket's source"
    );
    assert_eq!(resolved.max_output_tokens, None);
    assert_eq!(
        resolved.image_input,
        Some(sourced(true, ModelMetadataSource::Reported, NOW)),
        "a fact is never observed later than now"
    );
    assert_eq!(
        registry.observed_image_input(&other, None, NOW),
        ModelImageInputCapability::Unsupported
    );
    assert_eq!(registry.state.lock().routes.len(), 2);
}

#[test]
fn opening_drops_what_expired() {
    let (registry, store) = opened(ModelMetadataSnapshot::default());
    registry.learn_image_input_unsupported(&route(PROVIDER, "model"), NOW);
    let store = Arc::new(RecordingStore {
        loaded: last_saved(&store),
        saved: Mutex::default(),
    });

    let reopened = ModelMetadataRegistry::open(store, NOW + LEARNED_TTL_MS + 1);

    assert!(reopened.state.lock().routes.is_empty());
}

/// A refused reasoning parameter is learned as "no reasoning", beats the
/// listing's "yes", holds across a restart, and expires after 7 days.
#[test]
fn a_learned_reasoning_refusal_beats_a_reported_yes_and_is_kept_for_seven_days() {
    let (registry, store) = opened(ModelMetadataSnapshot::default());
    let model = route(PROVIDER, "model");
    let mut reasons = entry("model", None);
    reasons.reasoning = Some(true);
    registry.record_listing(&endpoint(PROVIDER), &[reasons], NOW);
    registry.learn_reasoning_unsupported(&model, NOW);

    let (reopened, _) = opened(last_saved(&store));
    let reasoning = |at: u64| {
        reopened
            .resolve(Some(&model), "model", ConfiguredModelFacts::default(), at)
            .reasoning
            .map(|fact| (fact.value, fact.source))
    };
    assert_eq!(
        reasoning(NOW + 1),
        Some((
            ModelReasoningSupport::Unsupported,
            ModelMetadataSource::Learned
        ))
    );
    assert_eq!(
        reasoning(NOW + LEARNED_TTL_MS + 1),
        Some((
            ModelReasoningSupport::Supported {
                efforts: Vec::new(),
                default: None,
            },
            ModelMetadataSource::Reported
        ))
    );
}
