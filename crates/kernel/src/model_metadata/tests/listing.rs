use super::*;

const PROVIDER: &str = "https://provider.test/v1";

#[test]
fn a_listing_keeps_each_entry_whole() {
    let registry = registry();
    let mut listed = entry("reasoner", Some(400_000));
    listed.max_output_tokens = Some(128_000);
    listed.image_input = Some(true);
    listed.reasoning = Some(true);
    listed.supported_reasoning_efforts =
        vec![ModelReasoningEffort::Low, ModelReasoningEffort::High];
    listed.default_reasoning_effort = Some(ModelReasoningEffort::High);
    let mut plain = entry("plain", None);
    plain.reasoning = Some(false);
    registry.record_listing(&endpoint(PROVIDER), &[listed, plain], NOW);

    let resolved = registry.resolve(
        Some(&route(PROVIDER, "reasoner")),
        "reasoner",
        Default::default(),
        NOW,
    );
    let reported = |value| sourced(value, ModelMetadataSource::Reported, NOW);
    assert_eq!(resolved.context_window, Some(reported(400_000)));
    assert_eq!(resolved.max_output_tokens, Some(reported(128_000)));
    assert_eq!(
        resolved.image_input,
        Some(sourced(true, ModelMetadataSource::Reported, NOW))
    );
    assert_eq!(
        resolved.reasoning,
        Some(sourced(
            ModelReasoningSupport::Supported {
                efforts: vec![ModelReasoningEffort::Low, ModelReasoningEffort::High],
                default: Some(ModelReasoningEffort::High),
            },
            ModelMetadataSource::Reported,
            NOW
        ))
    );
    let plain = registry.resolve(
        Some(&route(PROVIDER, "plain")),
        "plain",
        Default::default(),
        NOW,
    );
    assert_eq!(
        plain.reasoning,
        Some(sourced(
            ModelReasoningSupport::Unsupported,
            ModelMetadataSource::Reported,
            NOW
        ))
    );
    assert_eq!(plain.context_window, None);
}

#[test]
fn a_new_listing_replaces_what_it_reports_again() {
    let registry = registry();
    let local = route(PROVIDER, "local-model");
    registry.record_listing(
        &endpoint(PROVIDER),
        &[entry("local-model", Some(32_768))],
        NOW,
    );
    registry.record_listing(&endpoint(PROVIDER), &[entry("local-model", None)], NOW + 1);

    assert_eq!(window(&registry, None, &local, NOW + 1), None);
}

#[test]
fn a_model_left_out_of_a_listing_keeps_its_facts_until_they_expire() {
    let registry = registry();
    let kept = route(PROVIDER, "kept");
    let dropped = route(PROVIDER, "dropped");
    registry.record_listing(
        &endpoint(PROVIDER),
        &[
            entry("kept", Some(100_000)),
            entry("dropped", Some(200_000)),
        ],
        NOW,
    );
    registry.record_listing(&endpoint(PROVIDER), &[entry("kept", Some(100_000))], NOW);

    let expiry = NOW + REPORTED_TTL_MS;
    assert_eq!(window(&registry, None, &dropped, expiry), Some(200_000));
    assert_eq!(window(&registry, None, &dropped, expiry + 1), None);
    assert_eq!(
        window(&registry, None, &kept, expiry + 1),
        Some(100_000),
        "a model the latest listing names keeps what it reported"
    );
}

#[test]
fn a_listing_of_fewer_than_half_its_models_is_trusted_when_it_repeats() {
    let registry = registry();
    let provider = endpoint(PROVIDER);
    let survivor = route(PROVIDER, "a");
    registry.record_listing(
        &provider,
        &[
            entry("a", Some(100_000)),
            entry("b", Some(100_000)),
            entry("c", Some(100_000)),
        ],
        NOW,
    );

    registry.record_listing(&provider, &[entry("a", Some(50_000))], NOW + 1);
    assert_eq!(
        window(&registry, None, &survivor, NOW + 1),
        Some(100_000),
        "the shrunk listing is not trusted"
    );

    registry.record_listing(&provider, &[entry("a", Some(50_000))], NOW + 2);
    assert_eq!(window(&registry, None, &survivor, NOW + 2), Some(50_000));
    registry.record_listing(&provider, &[entry("a", Some(40_000))], NOW + 3);
    assert_eq!(window(&registry, None, &survivor, NOW + 3), Some(40_000));
}

#[test]
fn a_listing_of_as_many_other_models_is_trusted() {
    let registry = registry();
    let provider = endpoint(PROVIDER);
    registry.record_listing(&provider, &[entry("model-a", Some(100_000))], NOW);
    registry.record_listing(&provider, &[entry("model-b", Some(32_000))], NOW + 1);

    assert_eq!(
        window(&registry, None, &route(PROVIDER, "model-b"), NOW + 1),
        Some(32_000),
        "a single-model server restarted with another model is believed at once"
    );
}

#[test]
fn a_listing_that_loses_half_its_models_is_trusted() {
    let registry = registry();
    let provider = endpoint(PROVIDER);
    registry.record_listing(
        &provider,
        &[entry("a", Some(100_000)), entry("b", Some(100_000))],
        NOW,
    );
    registry.record_listing(&provider, &[entry("a", Some(50_000))], NOW + 1);

    assert_eq!(
        window(&registry, None, &route(PROVIDER, "a"), NOW + 1),
        Some(50_000)
    );
}

/// The ids of the recorded listing: none before one, then the listing's;
/// a listing the shrink guard holds leaves the ids before it, and the one
/// that confirms it replaces them.
#[test]
fn the_listed_models_are_the_recorded_listings_ids() {
    let registry = registry();
    let provider = endpoint(PROVIDER);
    assert_eq!(registry.listed_models(&provider), None);
    let ids = |ids: &[&str]| Some(ids.iter().map(|id| id.to_string()).collect());

    registry.record_listing(
        &provider,
        &[entry("b", None), entry("a", None), entry("c", None)],
        NOW,
    );
    assert_eq!(registry.listed_models(&provider), ids(&["a", "b", "c"]));
    assert_eq!(
        registry.listed_models(&endpoint("https://other.test/v1")),
        None
    );

    registry.record_listing(&provider, &[entry("a", None)], NOW + 1);
    assert_eq!(
        registry.listed_models(&provider),
        ids(&["a", "b", "c"]),
        "a held listing leaves the ids before it"
    );
    registry.record_listing(&provider, &[entry("a", None)], NOW + 2);
    assert_eq!(registry.listed_models(&provider), ids(&["a"]));
}

#[test]
fn the_same_endpoint_spelled_with_a_trailing_slash_is_one_route() {
    let registry = registry();
    registry.record_listing(
        &endpoint("https://provider.test/v1/"),
        &[entry("model", Some(64_000))],
        NOW,
    );

    assert_eq!(
        window(&registry, None, &route(PROVIDER, "model"), NOW),
        Some(64_000)
    );
}

#[test]
fn a_learned_refusal_is_route_scoped_and_expires() {
    let registry = registry();
    let target = route("https://one.test/v1", "model");
    registry.learn_image_input_unsupported(&target, NOW);

    assert_eq!(
        registry.observed_image_input(&target, None, NOW),
        ModelImageInputCapability::Unsupported
    );
    for other in [
        ModelEndpointKey::new(
            ModelApiProtocol::ChatCompletions,
            &ModelEndpoint::parse("https://one.test/v1").unwrap(),
        )
        .route("model"),
        route("https://two.test/v1", "model"),
        route("https://one.test/v1", "other"),
    ] {
        assert_eq!(
            registry.observed_image_input(&other, None, NOW),
            ModelImageInputCapability::Unknown
        );
    }
    assert_eq!(
        registry.observed_image_input(&target, None, NOW + LEARNED_TTL_MS),
        ModelImageInputCapability::Unsupported
    );
    assert_eq!(
        registry.observed_image_input(&target, None, NOW + LEARNED_TTL_MS + 1),
        ModelImageInputCapability::Unknown
    );
}

fn listed(
    registry: &ModelMetadataRegistry,
    entries: &[ModelCatalogEntry],
    now_ms: u64,
) -> Vec<ModelMetadata> {
    registry.resolve_listing(
        &endpoint(PROVIDER),
        entries,
        |_| ConfiguredModelFacts::default(),
        now_ms,
    )
}

#[test]
fn a_listing_resolves_each_entry_as_what_it_reports_not_what_was_recorded() {
    let registry = registry();
    registry.record_listing(
        &endpoint(PROVIDER),
        &[
            entry("grown", Some(100_000)),
            entry("silent", Some(100_000)),
        ],
        NOW,
    );

    let resolved = listed(
        &registry,
        &[entry("grown", Some(200_000)), entry("silent", None)],
        NOW + 1,
    );
    assert_eq!(
        resolved[0].context_window,
        Some(sourced(200_000, ModelMetadataSource::Reported, NOW + 1))
    );
    assert_eq!(
        resolved[1].context_window, None,
        "an entry that reports no window has none, whatever was recorded"
    );
}

#[test]
fn a_listing_keeps_what_the_refusals_taught_in_the_safe_direction() {
    let registry = registry();
    let refused = route(PROVIDER, "refused");
    registry.learn_image_input_unsupported(&refused, NOW);
    registry.learn_reasoning_unsupported(&refused, NOW);
    registry.learn_context_window(&refused, 50_000, NOW);
    registry.learn_context_window(&route(PROVIDER, "larger"), 300_000, NOW);
    let says_yes = |id: &str| ModelCatalogEntry {
        image_input: Some(true),
        reasoning: Some(true),
        ..entry(id, Some(200_000))
    };

    let resolved = listed(&registry, &[says_yes("refused"), says_yes("larger")], NOW);
    let learned = ModelMetadataSource::Learned;
    assert_eq!(resolved[0].image_input, Some(sourced(false, learned, NOW)));
    assert_eq!(
        resolved[0].reasoning,
        Some(sourced(ModelReasoningSupport::Unsupported, learned, NOW))
    );
    assert_eq!(
        resolved[0].context_window,
        Some(sourced(50_000, learned, NOW))
    );
    assert_eq!(
        resolved[1].context_window,
        Some(sourced(200_000, ModelMetadataSource::Reported, NOW)),
        "a learned window larger than the reported one does not count"
    );
    let expired = listed(&registry, &[says_yes("refused")], NOW + LEARNED_TTL_MS + 1);
    assert_eq!(expired[0].supports_image_input(), Some(true));
}

#[test]
fn a_listing_takes_what_the_person_configured_for_each_model_first() {
    let registry = registry();
    let resolved = registry.resolve_listing(
        &endpoint(PROVIDER),
        &[
            entry("configured", Some(200_000)),
            entry("plain", Some(200_000)),
        ],
        |model| match model {
            "configured" => ConfiguredModelFacts {
                context_window: Some(64_000),
                reasoning: Some(ModelReasoningSupport::Unsupported),
                ..ConfiguredModelFacts::default()
            },
            _ => ConfiguredModelFacts::default(),
        },
        NOW,
    );

    let configured = ModelMetadataSource::Configured;
    assert_eq!(
        resolved[0].context_window,
        Some(sourced(64_000, configured, 0))
    );
    assert_eq!(
        resolved[0].reasoning,
        Some(sourced(ModelReasoningSupport::Unsupported, configured, 0))
    );
    assert_eq!(
        resolved[1].context_window,
        Some(sourced(200_000, ModelMetadataSource::Reported, NOW))
    );
}

#[test]
fn a_listing_fills_from_models_dev_and_the_built_in_window() {
    let registry = registry();
    let document = serde_json::json!({"provider": {"models": {"gpt-5": {
        "reasoning": true,
        "modalities": {"input": ["text", "image"]},
        "limit": {"context": 400_000}
    }}}});
    registry.record_models_dev(
        hanafubuki_agent_model_api::ModelsDevCatalog::from_api_json(
            document.to_string().as_bytes(),
        )
        .unwrap(),
        NOW,
    );

    let resolved = listed(
        &registry,
        &[entry("gpt-5", None), entry("gpt-4o", None)],
        NOW,
    );
    assert_eq!(
        resolved[0].context_window,
        Some(sourced(400_000, ModelMetadataSource::Catalog, NOW))
    );
    assert_eq!(resolved[0].supports_reasoning(), Some(true));
    assert_eq!(resolved[0].supports_image_input(), Some(true));
    assert_eq!(
        resolved[1].context_window,
        Some(sourced(128_000, ModelMetadataSource::BuiltIn, 0))
    );
    assert_eq!(resolved[1].supports_reasoning(), None);
}

#[test]
fn a_listing_the_shrink_guard_held_still_resolves_from_its_entries() {
    let registry = registry();
    let provider = endpoint(PROVIDER);
    let entries: Vec<ModelCatalogEntry> = ["a", "b", "c", "d"]
        .into_iter()
        .map(|id| entry(id, Some(100_000)))
        .collect();
    registry.record_listing(&provider, &entries, NOW);
    let shrunk = [entry("a", Some(50_000))];
    registry.record_listing(&provider, &shrunk, NOW + 1);

    assert_eq!(
        window(&registry, None, &route(PROVIDER, "a"), NOW + 1),
        Some(100_000),
        "the store holds the listing before"
    );
    assert_eq!(
        listed(&registry, &shrunk, NOW + 1)[0].context_window,
        Some(sourced(50_000, ModelMetadataSource::Reported, NOW + 1))
    );
}

/// The largest listing, against a full registry and a models.dev copy of
/// 10,000 models, resolves well under a listing's own time: one lock, and
/// lookups per entry.
#[test]
fn a_large_listing_resolves_quickly() {
    let registry = registry();
    let full: Vec<ModelCatalogEntry> = (0..MAX_ROUTES)
        .map(|index| entry(&format!("other-{index}"), Some(64_000)))
        .collect();
    registry.record_listing(&endpoint("https://full.test/v1"), &full, NOW);
    let providers: serde_json::Map<String, serde_json::Value> = (0..100)
        .map(|provider| {
            let models: serde_json::Map<String, serde_json::Value> = (0..100)
                .map(|model| {
                    (
                        format!("model-{provider}-{model}"),
                        serde_json::json!({"reasoning": true, "limit": {"context": 128_000}}),
                    )
                })
                .collect();
            (
                format!("provider-{provider}"),
                serde_json::json!({ "models": models }),
            )
        })
        .collect();
    registry.record_models_dev(
        hanafubuki_agent_model_api::ModelsDevCatalog::from_api_json(
            serde_json::Value::Object(providers).to_string().as_bytes(),
        )
        .unwrap(),
        NOW,
    );
    let entries: Vec<ModelCatalogEntry> = (0..1_000)
        .map(|index| entry(&format!("model-{}-{}", index % 100, index / 100), None))
        .collect();

    let started = std::time::Instant::now();
    let resolved = listed(&registry, &entries, NOW);
    let elapsed = started.elapsed();

    assert_eq!(resolved.len(), 1_000);
    assert!(resolved
        .iter()
        .all(|metadata| metadata.supports_reasoning() == Some(true)));
    // Milliseconds here; the bound leaves room for a loaded debug build.
    assert!(elapsed < std::time::Duration::from_secs(5), "{elapsed:?}");
}

#[test]
fn the_routes_are_bounded_dropping_the_oldest() {
    let registry = registry();
    let oldest = route("https://oldest.test/v1", "model");
    registry.learn_image_input_unsupported(&oldest, NOW);
    let listing: Vec<ModelCatalogEntry> = (0..MAX_ROUTES)
        .map(|index| entry(&format!("model-{index}"), Some(64_000)))
        .collect();
    registry.record_listing(&endpoint("https://bounded.test/v1"), &listing, NOW + 1);

    assert_eq!(registry.state.lock().routes.len(), MAX_ROUTES);
    assert_eq!(
        registry.observed_image_input(&oldest, None, NOW + 1),
        ModelImageInputCapability::Unknown,
        "the route observed longest ago is dropped"
    );
    let newest = route("https://newest.test/v1", "model");
    registry.learn_image_input_unsupported(&newest, NOW + 1);
    assert_eq!(registry.state.lock().routes.len(), MAX_ROUTES);
    assert_eq!(
        registry.observed_image_input(&newest, None, NOW + 1),
        ModelImageInputCapability::Unsupported,
        "the route just learned is never the one dropped"
    );
}
