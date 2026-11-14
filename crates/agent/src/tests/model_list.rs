//! `list_provider_models` against fake Providers: each listed model comes
//! with what is known of it, resolved as `model_metadata` resolves it with
//! the listing's own entry as what was reported, whatever the source of a
//! fact; and one Provider's listing answers while another's is still
//! pending, so a caller can ask one Provider at a time.

use hanafubuki_agent_config::AgentModelOverrideSettings;
use hanafubuki_agent_kernel::{ModelMetadata, ModelMetadataSource, ModelsDevCatalog, Sourced};

use super::*;

/// An OpenAI-shaped listing (an id and little else) with an
/// OpenRouter-shaped entry among it.
const LISTING: &str = r#"{"data":[
    {"id":"gpt-5.1","object":"model","owned_by":"openai"},
    {"id":"configured-model","context_length":200000,"supported_parameters":["reasoning"]},
    {"id":"vendor/router-model","context_length":131072,
     "top_provider":{"max_completion_tokens":8192},
     "architecture":{"input_modalities":["text","image"]},
     "supported_parameters":["reasoning"]},
    {"id":"catalogued"}
]}"#;

fn stop(service: &AgentApplicationService) {
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
}

/// `metadata` with no fact's observation time: two reads of the clock
/// stamp the same reported fact apart.
fn unstamped(metadata: &ModelMetadata) -> ModelMetadata {
    fn fact<T: Clone>(fact: &Option<Sourced<T>>) -> Option<Sourced<T>> {
        fact.clone().map(|fact| Sourced {
            observed_at_ms: 0,
            ..fact
        })
    }
    ModelMetadata {
        context_window: fact(&metadata.context_window),
        max_output_tokens: fact(&metadata.max_output_tokens),
        reasoning: fact(&metadata.reasoning),
        image_input: fact(&metadata.image_input),
        structured_output: fact(&metadata.structured_output),
    }
}

fn window(metadata: &ModelMetadata) -> Option<(u64, ModelMetadataSource)> {
    metadata
        .context_window
        .as_ref()
        .map(|window| (window.value, window.source))
}

/// One call gives each model its facts from every source: the built-in
/// window for an OpenAI model that reports none, the model's own settings
/// over what it reports, what an OpenRouter-shaped entry reports, and what
/// models.dev says; each the same as `model_metadata` gives it once the
/// listing is recorded.
#[tokio::test]
async fn each_listed_model_carries_its_resolved_facts() {
    let server = FakeServer::with_listings(Vec::new(), vec![FakeListing::ok(LISTING)]).await;
    let (service, paths) = service_with("model-list-facts", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        config
            .mutate_providers(1, |registry| {
                registry.providers[0].models.insert(
                    "configured-model".into(),
                    AgentModelOverrideSettings {
                        context_window_tokens: Some(64_000),
                        reasoning_efforts: Some(Vec::new()),
                        ..AgentModelOverrideSettings::default()
                    },
                );
                Ok(())
            })
            .unwrap();
    });
    let catalog = serde_json::json!({"elsewhere": {"models": {"catalogued": {
        "reasoning": true,
        "limit": {"context": 500_000}
    }}}});
    service.core.record_models_dev_catalog(
        ModelsDevCatalog::from_api_json(catalog.to_string().as_bytes()).unwrap(),
    );

    let results = service
        .list_provider_models(AgentProviderModelCatalogRequest::default())
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].error, None);
    let models = &results[0].models;
    let model = |id: &str| {
        models
            .iter()
            .find(|model| model.entry.id == id)
            .unwrap_or_else(|| panic!("{id} is not listed"))
    };
    let openai = model("gpt-5.1");
    assert_eq!(openai.entry.context_window, None);
    assert_eq!(
        window(&openai.metadata),
        Some((272_000, ModelMetadataSource::BuiltIn))
    );
    assert_eq!(openai.metadata.supports_reasoning(), None);
    assert_eq!(openai.metadata.supports_image_input(), None);

    let configured = model("configured-model");
    assert_eq!(
        window(&configured.metadata),
        Some((64_000, ModelMetadataSource::Configured))
    );
    assert_eq!(configured.metadata.supports_reasoning(), Some(false));
    assert_eq!(
        configured.metadata.reasoning.as_ref().unwrap().source,
        ModelMetadataSource::Configured
    );

    let routed = model("vendor/router-model");
    assert_eq!(
        window(&routed.metadata),
        Some((131_072, ModelMetadataSource::Reported))
    );
    assert_eq!(
        routed
            .metadata
            .max_output_tokens
            .as_ref()
            .map(|fact| (fact.value, fact.source)),
        Some((8_192, ModelMetadataSource::Reported))
    );
    assert_eq!(routed.metadata.supports_reasoning(), Some(true));
    assert_eq!(routed.metadata.supports_image_input(), Some(true));

    let catalogued = model("catalogued");
    assert_eq!(
        window(&catalogued.metadata),
        Some((500_000, ModelMetadataSource::Catalog))
    );
    assert_eq!(catalogued.metadata.supports_reasoning(), Some(true));

    for listed in models {
        let known = service
            .model_metadata("provider-1", &listed.entry.id)
            .await
            .unwrap();
        assert_eq!(
            unstamped(&listed.metadata),
            unstamped(&known),
            "{}",
            listed.entry.id
        );
    }
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A result's JSON is each entry's own fields, flat as before, with its
/// facts beside them under `metadata`.
#[tokio::test]
async fn a_listed_models_json_is_its_entry_with_its_metadata_beside() {
    let server = FakeServer::with_listings(
        Vec::new(),
        vec![FakeListing::ok(
            r#"{"data":[{"id":"fixture-model","context_length":123456}]}"#,
        )],
    )
    .await;
    let (service, paths) = configured_service("model-list-json", &server.base_url);

    let results = service
        .list_provider_models(AgentProviderModelCatalogRequest::default())
        .await
        .unwrap();

    let json = serde_json::to_value(&results).unwrap();
    let model = &json[0]["models"][0];
    assert_eq!(model["id"], "fixture-model");
    assert_eq!(model["contextWindow"], 123_456);
    assert!(model.get("displayName").is_some(), "{model}");
    assert!(model.get("entry").is_none(), "{model}");
    assert_eq!(model["metadata"]["contextWindow"]["value"], 123_456);
    assert_eq!(model["metadata"]["contextWindow"]["source"], "reported");
    assert_eq!(model["metadata"]["imageInput"], serde_json::Value::Null);
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The premise of listing one Provider at a time: while the first
/// Provider's listing is held, a request for the second alone answers with
/// its models and their facts; releasing the first then answers its own
/// request.
#[tokio::test]
async fn one_providers_listing_answers_while_anothers_is_pending() {
    let release = Arc::new(tokio::sync::Notify::new());
    let slow = FakeServer::with_listings(
        Vec::new(),
        vec![FakeListing::ok(r#"{"data":[{"id":"slow-model"}]}"#).held(release.clone())],
    )
    .await;
    let fast = FakeServer::with_listings(
        Vec::new(),
        vec![FakeListing::ok(
            r#"{"data":[{"id":"fast-model","context_length":32768}]}"#,
        )],
    )
    .await;
    let (service, paths) = service_with("model-list-independent", |_, config| {
        configure_test_provider(config, &slow.base_url, None);
        add_test_provider(config, "provider-2", "Fast", &fast.base_url);
    });
    let only = |provider_id: &str| AgentProviderModelCatalogRequest {
        provider_ids: Some(vec![provider_id.to_string()]),
    };

    let pending = tokio::spawn({
        let service = service.clone();
        let request = only("provider-1");
        async move { service.list_provider_models(request).await }
    });
    let deadline = Instant::now() + TEST_WAIT;
    while slow.listing_requests().is_empty() {
        assert!(Instant::now() < deadline, "the slow listing was not asked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let answered = service
        .list_provider_models(only("provider-2"))
        .await
        .unwrap();
    assert!(!pending.is_finished(), "the slow listing is still held");
    assert_eq!(answered.len(), 1);
    assert_eq!(answered[0].provider_id, "provider-2");
    assert_eq!(answered[0].error, None);
    assert_eq!(answered[0].models[0].entry.id, "fast-model");
    assert_eq!(
        window(&answered[0].models[0].metadata),
        Some((32_768, ModelMetadataSource::Reported))
    );

    release.notify_one();
    let first = tokio::time::timeout(TEST_WAIT, pending)
        .await
        .expect("the released listing answers")
        .unwrap()
        .unwrap();
    assert_eq!(first[0].provider_id, "provider-1");
    assert_eq!(first[0].models[0].entry.id, "slow-model");
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
