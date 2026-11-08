use std::time::Duration;

use hanafubuki_agent_web_search::{
    WebSearchConfig, WebSearchError, WebSearchProviderPreference, WebSearchService,
};

#[test]
fn search_endpoint_errors_name_only_the_search_endpoints() {
    assert_eq!(WebSearchConfig::default().search_endpoint_error(), None);

    let mut config = WebSearchConfig::default();
    config.searxng_base_url = Some("not a url".to_string());
    assert_eq!(
        config.search_endpoint_error(),
        Some(WebSearchError::InvalidConfiguration(
            "search endpoint is not a URL"
        ))
    );

    let mut config = WebSearchConfig::default();
    config.provider_preference = WebSearchProviderPreference::Searxng;
    assert!(config.search_endpoint_error().is_some());

    // A fault outside the endpoints is not a search endpoint error.
    let mut config = WebSearchConfig::default();
    config.maximum_concurrent_requests = 0;
    assert_eq!(config.search_endpoint_error(), None);
}

#[test]
fn configuration_rejects_unbounded_runtime_limits() {
    let mut config = WebSearchConfig::default();
    config.maximum_concurrent_requests = 0;
    assert!(WebSearchService::new(config).is_err());

    let mut config = WebSearchConfig::default();
    config.request_timeout = Duration::from_secs(61);
    assert!(WebSearchService::new(config).is_err());

    let mut config = WebSearchConfig::default();
    config.jina_reader_timeout = Duration::from_secs(61);
    assert!(WebSearchService::new(config).is_err());

    let mut config = WebSearchConfig::default();
    config.jina_reader_enabled = true;
    config.page_fetch_total_timeout = Duration::from_secs(39);
    assert!(WebSearchService::new(config).is_err());

    let mut config = WebSearchConfig::default();
    config.page_fetch_total_timeout = Duration::from_secs(121);
    assert!(WebSearchService::new(config).is_err());
}
