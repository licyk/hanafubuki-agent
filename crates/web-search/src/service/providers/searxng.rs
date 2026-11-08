use super::super::*;

impl WebSearchService {
    pub(in crate::service) async fn search_searxng(
        &self,
        request: &WebSearchRequest,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let base = self
            .inner
            .searxng_url
            .as_ref()
            .ok_or(WebSearchError::InvalidConfiguration(
                "SearXNG endpoint is not configured",
            ))?;
        let mut url = base.clone();
        let base_path = base.path().trim_end_matches('/');
        url.set_path(&format!("{base_path}/search"));
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("q", &request.query)
                .append_pair("format", "json")
                .append_pair("categories", request.category.as_searxng())
                .append_pair("pageno", "1");
            if let Some(time_range) = request.time_range {
                pairs.append_pair("time_range", time_range.as_searxng());
            }
        }
        let body = self.get_bounded(url, cancellation).await?;
        let decoded: SearxResponse =
            serde_json::from_slice(&body).map_err(|_| WebSearchError::InvalidResponse)?;
        let results = normalize_results(
            decoded.results.into_iter().map(|result| WebSearchResult {
                title: result.title,
                url: result.url,
                snippet: result.content.unwrap_or_default(),
                source: result
                    .engines
                    .as_ref()
                    .and_then(|engines| engines.first())
                    .cloned()
                    .or(result.engine)
                    .unwrap_or_else(|| "searxng".to_string()),
                engines: result
                    .engines
                    .unwrap_or_else(|| vec!["searxng".to_string()]),
                published_at: result.published_date,
            }),
            request.maximum_results,
        );
        Ok(WebSearchResponse {
            query: request.query.clone(),
            providers: vec![WebSearchProvider::Searxng],
            results,
            answers: bounded_strings(
                decoded.answers.unwrap_or_default(),
                MAX_WEB_SEARCH_ANSWERS,
                MAX_WEB_SEARCH_ANSWER_BYTES,
            ),
            suggestions: bounded_strings(
                decoded.suggestions.unwrap_or_default(),
                MAX_WEB_SEARCH_SUGGESTIONS,
                MAX_WEB_SEARCH_TITLE_BYTES,
            ),
            partial_failures: Vec::new(),
            cache_hit: false,
        })
    }
}

#[derive(Deserialize)]
struct SearxResponse {
    #[serde(default)]
    results: Vec<SearxResult>,
    answers: Option<Vec<String>>,
    suggestions: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct SearxResult {
    title: String,
    url: String,
    content: Option<String>,
    engine: Option<String>,
    engines: Option<Vec<String>>,
    #[serde(rename = "publishedDate", alias = "published_date")]
    published_date: Option<String>,
}
