use super::*;

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

#[test]
fn the_configured_window_overrides_the_provider_and_the_table() {
    let catalog = ModelCatalog::default();
    catalog.record_listing("https://provider.test/v1", &[entry("gpt-5", Some(400_000))]);

    assert_eq!(
        catalog.context_window(Some(64_000), "https://provider.test/v1", "gpt-5"),
        Some(64_000)
    );
}

#[test]
fn a_reported_window_applies_to_its_provider_only() {
    let catalog = ModelCatalog::default();
    catalog.record_listing(
        "https://provider.test/v1",
        &[
            entry("gpt-5", Some(400_000)),
            entry("local-model", Some(32_768)),
        ],
    );

    assert_eq!(
        catalog.context_window(None, "https://provider.test/v1", "gpt-5"),
        Some(400_000)
    );
    assert_eq!(
        catalog.context_window(None, "https://provider.test/v1", "local-model"),
        Some(32_768)
    );
    assert_eq!(
        catalog.context_window(None, "https://other.test/v1", "gpt-5"),
        Some(272_000)
    );
    assert_eq!(
        catalog.context_window(None, "https://other.test/v1", "local-model"),
        None
    );
}

#[test]
fn a_new_listing_replaces_what_the_provider_reported_before() {
    let catalog = ModelCatalog::default();
    catalog.record_listing(
        "https://provider.test/v1",
        &[entry("local-model", Some(32_768))],
    );
    catalog.record_listing("https://provider.test/v1", &[entry("local-model", None)]);

    assert_eq!(
        catalog.context_window(None, "https://provider.test/v1", "local-model"),
        None
    );
}

#[test]
fn the_built_in_table_matches_the_longest_prefix() {
    assert_eq!(built_in_context_window("claude-haiku-4-5"), Some(200_000));
    assert_eq!(built_in_context_window("claude-opus-5-5"), Some(1_000_000));
    assert_eq!(built_in_context_window("GPT-4o-mini"), Some(128_000));
    assert_eq!(
        built_in_context_window("openrouter/anthropic/claude-sonnet-4-6"),
        Some(1_000_000)
    );
}

#[test]
fn an_unknown_model_has_no_built_in_window() {
    for model in ["", "fixture-model", "gpt-codex", "llama3.1:8b"] {
        assert_eq!(built_in_context_window(model), None, "{model}");
    }
}
