use super::*;

#[test]
fn a_model_id_is_compared_lowercase_without_vendor_latest_or_date() {
    for (id, normalized) in [
        ("gpt-4o", "gpt-4o"),
        ("GPT-4o", "gpt-4o"),
        ("openai/gpt-4o", "gpt-4o"),
        ("openrouter/anthropic/claude-sonnet-4", "claude-sonnet-4"),
        ("claude-3-5-sonnet-20241022", "claude-3-5-sonnet"),
        ("claude-3-5-sonnet-latest", "claude-3-5-sonnet"),
        ("anthropic/Claude-3-5-Haiku-20241022", "claude-3-5-haiku"),
        // Only an eight-digit date, and only at the end.
        ("gpt-4o-2024-08-06", "gpt-4o-2024-08-06"),
        ("model-2024102", "model-2024102"),
        ("model-20241022-preview", "model-20241022-preview"),
        (" deepseek-chat ", "deepseek-chat"),
    ] {
        assert_eq!(normalized_model_id(id), normalized, "{id}");
    }
}

#[test]
fn a_provider_is_named_by_its_id_a_word_of_it_or_its_host() {
    let names = |id: &str, base_url: &str| {
        ModelProviderNames::new(id, &ModelEndpoint::parse(base_url).unwrap())
    };
    let provider = |api_host: Option<&str>| ModelsDevProvider {
        api_host: api_host.map(str::to_owned),
        models: BTreeMap::new(),
    };
    let proxy = names("Anthropic", "http://127.0.0.1:4000/v1");
    assert!(proxy.own("anthropic", &provider(None)));
    assert!(!proxy.own("openai", &provider(None)));
    assert!(names("my-openai-proxy", "http://localhost/v1").own("openai", &provider(None)));
    let deepseek = names("ds", "https://api.deepseek.com/v1");
    assert!(deepseek.own("deepseek", &provider(None)));
    assert!(deepseek.own("elsewhere", &provider(Some("api.deepseek.com"))));
    assert!(!deepseek.own("elsewhere", &provider(Some("api.other.com"))));
}
