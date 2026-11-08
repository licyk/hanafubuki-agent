use super::normalize::empty_response;
use super::WebSearchService;
use crate::model::{
    WebSearchError, WebSearchFailure, WebSearchFailureCode, WebSearchProvider,
    WebSearchProviderPreference, WebSearchRequest, WebSearchResponse,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;

impl WebSearchService {
    pub(super) async fn execute(
        &self,
        request: &WebSearchRequest,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        // Rust-owned settings are policy, not a model suggestion. An explicit
        // configured provider cannot be overridden by the tool arguments;
        // only Auto lets the model narrow the fixed/trusted provider set.
        let provider = match self.inner.config.provider_preference {
            WebSearchProviderPreference::Auto => request.provider,
            WebSearchProviderPreference::Duckduckgo => WebSearchProvider::Duckduckgo,
            WebSearchProviderPreference::Searxng => WebSearchProvider::Searxng,
        };
        match provider {
            WebSearchProvider::Duckduckgo => {
                self.single_provider(WebSearchProvider::Duckduckgo, request, cancellation)
                    .await
            }
            WebSearchProvider::Searxng => {
                if self.inner.searxng_url.is_none() {
                    return Err(WebSearchError::InvalidConfiguration(
                        "SearXNG was requested but no endpoint is configured",
                    ));
                }
                self.single_provider(WebSearchProvider::Searxng, request, cancellation)
                    .await
            }
            WebSearchProvider::Auto => {
                let mut failures = Vec::new();
                if self.inner.searxng_url.is_some() {
                    match self
                        .fetch_provider(WebSearchProvider::Searxng, request, cancellation)
                        .await
                    {
                        Ok(response) if !response.results.is_empty() => return Ok(response),
                        Ok(_) => failures.push(WebSearchFailure {
                            provider: WebSearchProvider::Searxng,
                            code: WebSearchFailureCode::NoResults,
                        }),
                        Err(WebSearchError::Cancelled) => return Err(WebSearchError::Cancelled),
                        Err(error) => failures.push(WebSearchFailure {
                            provider: WebSearchProvider::Searxng,
                            code: error.failure_code(),
                        }),
                    }
                }
                match self
                    .fetch_provider(WebSearchProvider::Duckduckgo, request, cancellation)
                    .await
                {
                    Ok(mut response) => {
                        if self.inner.searxng_url.is_some() {
                            response.providers.insert(0, WebSearchProvider::Searxng);
                        }
                        response.partial_failures.splice(0..0, failures);
                        Ok(response)
                    }
                    Err(WebSearchError::Cancelled) => Err(WebSearchError::Cancelled),
                    Err(error) => {
                        failures.push(WebSearchFailure {
                            provider: WebSearchProvider::Duckduckgo,
                            code: error.failure_code(),
                        });
                        Ok(empty_response(request, failures))
                    }
                }
            }
        }
    }

    async fn single_provider(
        &self,
        provider: WebSearchProvider,
        request: &WebSearchRequest,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        match self.fetch_provider(provider, request, cancellation).await {
            Ok(response) => Ok(response),
            Err(WebSearchError::Cancelled) => Err(WebSearchError::Cancelled),
            Err(error) => Ok(empty_response(
                request,
                vec![WebSearchFailure {
                    provider,
                    code: error.failure_code(),
                }],
            )),
        }
    }

    async fn fetch_provider(
        &self,
        provider: WebSearchProvider,
        request: &WebSearchRequest,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        match provider {
            WebSearchProvider::Duckduckgo => self.search_duckduckgo(request, cancellation).await,
            WebSearchProvider::Searxng => self.search_searxng(request, cancellation).await,
            WebSearchProvider::Auto => Err(WebSearchError::InvalidConfiguration(
                "auto is not a concrete provider",
            )),
        }
    }
}
