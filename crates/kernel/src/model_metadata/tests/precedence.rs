use super::*;

use hanafubuki_agent_model_api::ModelMetadata;

const PROVIDER: &str = "https://provider.test/v1";

fn learn_window(registry: &ModelMetadataRegistry, route: &ModelRouteKey, tokens: u64) {
    registry.state.lock().learn(route, NOW, |learned| {
        learned.context_window = Some(sourced(tokens, ModelMetadataSource::Learned, NOW));
    });
}

#[test]
fn the_configured_window_overrides_every_other_source() {
    let registry = registry();
    let gpt = route(PROVIDER, "gpt-5");
    registry.record_listing(&endpoint(PROVIDER), &[entry("gpt-5", Some(400_000))], NOW);
    learn_window(&registry, &gpt, 100_000);

    let resolved = registry.resolve(
        Some(&gpt),
        "gpt-5",
        ConfiguredModelFacts {
            context_window: Some(64_000),
            image_input: None,
        },
        NOW,
    );

    assert_eq!(
        resolved.context_window,
        Some(sourced(64_000, ModelMetadataSource::Configured, 0))
    );
}

#[test]
fn a_reported_window_applies_to_its_route_only() {
    let registry = registry();
    registry.record_listing(
        &endpoint(PROVIDER),
        &[
            entry("gpt-5", Some(400_000)),
            entry("local-model", Some(32_768)),
        ],
        NOW,
    );

    assert_eq!(
        window(&registry, None, &route(PROVIDER, "gpt-5"), NOW),
        Some(400_000)
    );
    assert_eq!(
        window(&registry, None, &route(PROVIDER, "local-model"), NOW),
        Some(32_768)
    );
    let other = "https://other.test/v1";
    assert_eq!(
        window(&registry, None, &route(other, "gpt-5"), NOW),
        Some(272_000),
        "another Provider's gpt-5 has the built-in window"
    );
    assert_eq!(
        window(&registry, None, &route(other, "local-model"), NOW),
        None
    );
    let chat = ModelEndpointKey::new(
        ModelApiProtocol::ChatCompletions,
        &ModelEndpoint::parse(PROVIDER).unwrap(),
    )
    .route("local-model");
    assert_eq!(window(&registry, None, &chat, NOW), None);
    assert_eq!(
        registry.context_window(None, None, "local-model", NOW),
        None,
        "a transport without a route has the configured and built-in windows only"
    );
}

#[test]
fn a_learned_window_only_lowers_the_window() {
    let registry = registry();
    let listed = route(PROVIDER, "listed");
    let larger = route(PROVIDER, "larger");
    let unknown = route(PROVIDER, "unknown");
    let built_in = route(PROVIDER, "gpt-5");
    registry.record_listing(
        &endpoint(PROVIDER),
        &[
            entry("listed", Some(200_000)),
            entry("larger", Some(200_000)),
        ],
        NOW,
    );
    learn_window(&registry, &listed, 150_000);
    learn_window(&registry, &larger, 250_000);
    learn_window(&registry, &unknown, 90_000);
    learn_window(&registry, &built_in, 300_000);

    let learned = registry.resolve(Some(&listed), "listed", Default::default(), NOW);
    assert_eq!(
        learned.context_window,
        Some(sourced(150_000, ModelMetadataSource::Learned, NOW))
    );
    let reported = registry.resolve(Some(&larger), "larger", Default::default(), NOW);
    assert_eq!(
        reported.context_window,
        Some(sourced(200_000, ModelMetadataSource::Reported, NOW))
    );
    assert_eq!(window(&registry, None, &unknown, NOW), Some(90_000));
    assert_eq!(
        registry
            .resolve(Some(&built_in), "gpt-5", Default::default(), NOW)
            .context_window,
        Some(sourced(272_000, ModelMetadataSource::BuiltIn, 0))
    );
}

#[test]
fn a_learned_refusal_beats_a_reported_yes() {
    let registry = registry();
    let model = route(PROVIDER, "vision");
    let mut listed = entry("vision", None);
    listed.image_input = Some(true);
    listed.reasoning = Some(true);
    listed.supported_reasoning_efforts = vec![ModelReasoningEffort::Low];
    registry.record_listing(&endpoint(PROVIDER), &[listed], NOW);
    let reported = registry.resolve(Some(&model), "vision", Default::default(), NOW);
    assert_eq!(
        reported.image_input,
        Some(sourced(true, ModelMetadataSource::Reported, NOW))
    );
    assert_eq!(
        registry.observed_image_input(&model, NOW),
        ModelImageInputCapability::Unknown,
        "what a listing reports is not acted on yet"
    );

    registry.learn_image_input_unsupported(&model, NOW + 1);
    registry.state.lock().learn(&model, NOW + 1, |learned| {
        learned.reasoning = Some(sourced(
            ModelReasoningSupport::Unsupported,
            ModelMetadataSource::Learned,
            NOW + 1,
        ));
    });

    let resolved = registry.resolve(Some(&model), "vision", Default::default(), NOW + 1);
    assert_eq!(
        resolved.image_input,
        Some(sourced(false, ModelMetadataSource::Learned, NOW + 1))
    );
    assert_eq!(
        resolved.reasoning,
        Some(sourced(
            ModelReasoningSupport::Unsupported,
            ModelMetadataSource::Learned,
            NOW + 1
        ))
    );
    assert_eq!(
        registry.observed_image_input(&model, NOW + 1),
        ModelImageInputCapability::Unsupported
    );
    let configured = registry.resolve(
        Some(&model),
        "vision",
        ConfiguredModelFacts {
            context_window: None,
            image_input: Some(true),
        },
        NOW + 1,
    );
    assert_eq!(
        configured.image_input,
        Some(sourced(true, ModelMetadataSource::Configured, 0))
    );
}

#[test]
fn a_catalog_only_adds_what_nothing_else_says() {
    let registry = registry();
    let listed = route(PROVIDER, "listed");
    let unlisted = route(PROVIDER, "gpt-5");
    let refused = route(PROVIDER, "refused");
    let mut reported = entry("listed", Some(128_000));
    reported.image_input = Some(false);
    registry.record_listing(&endpoint(PROVIDER), &[reported], NOW);
    registry.learn_image_input_unsupported(&refused, NOW);
    let catalog = ModelMetadata {
        context_window: Some(sourced(400_000, ModelMetadataSource::Catalog, NOW)),
        image_input: Some(sourced(true, ModelMetadataSource::Catalog, NOW)),
        reasoning: Some(sourced(
            ModelReasoningSupport::Unsupported,
            ModelMetadataSource::Catalog,
            NOW,
        )),
        ..ModelMetadata::default()
    };
    for route in [&listed, &unlisted, &refused] {
        set_catalog(&registry, route, catalog.clone());
    }

    let behind_reported = registry.resolve(Some(&listed), "listed", Default::default(), NOW);
    assert_eq!(
        behind_reported.context_window.map(|fact| fact.source),
        Some(ModelMetadataSource::Reported)
    );
    assert_eq!(
        behind_reported.image_input,
        Some(sourced(false, ModelMetadataSource::Reported, NOW)),
        "a catalog's yes never contradicts a reported no"
    );
    let alone = registry.resolve(Some(&unlisted), "gpt-5", Default::default(), NOW);
    assert_eq!(
        alone.context_window,
        Some(sourced(400_000, ModelMetadataSource::Catalog, NOW)),
        "a catalog's window comes before the built-in one"
    );
    assert_eq!(
        alone.image_input,
        Some(sourced(true, ModelMetadataSource::Catalog, NOW))
    );
    assert_eq!(alone.reasoning, None, "a catalog's no is ignored");
    let after_refusal = registry.resolve(Some(&refused), "refused", Default::default(), NOW);
    assert_eq!(
        after_refusal.image_input,
        Some(sourced(false, ModelMetadataSource::Learned, NOW))
    );
}

#[test]
fn the_built_in_table_matches_the_longest_prefix() {
    assert_eq!(built_in::context_window("claude-haiku-4-5"), Some(200_000));
    assert_eq!(built_in::context_window("claude-opus-5-5"), Some(1_000_000));
    assert_eq!(built_in::context_window("GPT-4o-mini"), Some(128_000));
    assert_eq!(
        built_in::context_window("openrouter/anthropic/claude-sonnet-4-6"),
        Some(1_000_000)
    );
}

#[test]
fn an_unknown_model_has_no_built_in_window() {
    for model in ["", "fixture-model", "gpt-codex", "llama3.1:8b"] {
        assert_eq!(built_in::context_window(model), None, "{model}");
    }
}
