//! The structured-output rung a route's refusals taught: only ever lowered,
//! kept for a week, saved, and forgotten with the other learned facts.

use hanafubuki_agent_model_api::ModelStructuredOutputCapability as Rung;

use super::*;

const PROVIDER: &str = "https://gateway.test/v1";

#[test]
fn a_learned_rung_is_only_ever_lowered_while_it_holds() {
    let registry = registry();
    let model = route(PROVIDER, "model");
    assert_eq!(registry.structured_output(&model, NOW), None);

    registry.learn_structured_output(&model, Rung::Text, NOW);
    // A request that started before the text rung was learned may learn
    // the forced tool after it: the lower rung stays.
    registry.learn_structured_output(&model, Rung::RequiredTool, NOW + 1);
    assert_eq!(
        registry.structured_output(&model, NOW + 2),
        Some(Rung::Text)
    );
    let resolved = registry.resolve(
        Some(&model),
        model.model(),
        ConfiguredModelFacts::default(),
        NOW + 2,
    );
    assert_eq!(
        resolved.structured_output,
        Some(Sourced {
            value: Rung::Text,
            source: ModelMetadataSource::Learned,
            observed_at_ms: NOW,
        })
    );
    assert_eq!(
        registry.structured_output(&route(PROVIDER, "other"), NOW + 2),
        None,
        "per route"
    );
}

#[test]
fn a_learned_rung_expires_after_a_week_and_is_saved() {
    let store = Arc::new(RecordingStore::default());
    let registry = ModelMetadataRegistry::open(store.clone(), NOW);
    let model = route(PROVIDER, "model");
    registry.learn_structured_output(&model, Rung::RequiredTool, NOW);

    let reopened = ModelMetadataRegistry::open(
        Arc::new(RecordingStore {
            loaded: store.saved.lock().last().cloned().unwrap(),
            saved: Mutex::default(),
        }),
        NOW + 1,
    );
    assert_eq!(
        reopened.structured_output(&model, NOW + LEARNED_TTL_MS),
        Some(Rung::RequiredTool),
        "the next run starts at the learned rung"
    );
    assert_eq!(
        reopened.structured_output(&model, NOW + LEARNED_TTL_MS + 1),
        None,
        "a week later the protocol's rung is tried again"
    );

    // Once expired, a refusal teaches again from the protocol's rung.
    let expired = NOW + LEARNED_TTL_MS + 1;
    registry.learn_structured_output(&model, Rung::RequiredTool, expired);
    assert_eq!(
        registry.structured_output(&model, expired),
        Some(Rung::RequiredTool)
    );
}

#[test]
fn forgetting_drops_the_learned_rung() {
    let registry = registry();
    let model = route(PROVIDER, "model");
    registry.learn_structured_output(&model, Rung::Text, NOW);

    assert_eq!(
        registry.forget_learned(&endpoint(PROVIDER), Some("model"), NOW + 1),
        1
    );
    assert_eq!(registry.structured_output(&model, NOW + 1), None);
}
