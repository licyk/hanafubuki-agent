use super::*;

#[test]
fn an_unusable_search_endpoint_is_removed_with_the_reason() {
    let mut config = WebSearchConfig::default();
    config.provider_preference = WebSearchProviderPreference::Searxng;

    let (config, reason) = without_unusable_search(config);

    assert_eq!(
        reason.as_deref(),
        Some("web search is not configured: SearXNG is selected but no endpoint is configured")
    );
    assert_eq!(config.searxng_base_url, None);
    assert_eq!(
        config.provider_preference,
        WebSearchProviderPreference::Auto
    );
    assert!(WebSearchService::new(config).is_ok());
}

#[test]
fn a_usable_configuration_is_kept_as_it_is() {
    let mut config = WebSearchConfig::default();
    config.searxng_base_url = Some("https://search.example.com".to_string());

    let (kept, reason) = without_unusable_search(config.clone());

    assert_eq!(reason, None);
    assert_eq!(kept.searxng_base_url, config.searxng_base_url);
}
