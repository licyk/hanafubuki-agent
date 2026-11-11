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
