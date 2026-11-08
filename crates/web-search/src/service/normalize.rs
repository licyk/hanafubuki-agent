use crate::model::{
    WebSearchFailure, WebSearchRequest, WebSearchResponse, WebSearchResult,
    MAX_WEB_SEARCH_SNIPPET_BYTES, MAX_WEB_SEARCH_TITLE_BYTES, MAX_WEB_SEARCH_URL_BYTES,
};
use hanafubuki_agent_foundation::url_policy::UrlPolicy;
use std::collections::HashSet;

pub(super) fn normalize_results(
    results: impl IntoIterator<Item = WebSearchResult>,
    maximum_results: usize,
) -> Vec<WebSearchResult> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for result in results {
        let Some(url) = normalize_result_url(&result.url) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = truncate_utf8(result.title.trim(), MAX_WEB_SEARCH_TITLE_BYTES);
        if title.is_empty() {
            continue;
        }
        let mut engines = result
            .engines
            .into_iter()
            .map(|engine| truncate_utf8(engine.trim(), 64))
            .filter(|engine| !engine.is_empty())
            .take(8)
            .collect::<Vec<_>>();
        engines.sort();
        engines.dedup();
        normalized.push(WebSearchResult {
            title,
            url,
            snippet: truncate_utf8(result.snippet.trim(), MAX_WEB_SEARCH_SNIPPET_BYTES),
            source: truncate_utf8(result.source.trim(), 256),
            engines,
            published_at: result
                .published_at
                .map(|date| truncate_utf8(date.trim(), 128))
                .filter(|date| !date.is_empty()),
        });
        if normalized.len() == maximum_results {
            break;
        }
    }
    normalized
}

pub(super) fn normalize_result_url(raw: &str) -> Option<String> {
    let mut url = UrlPolicy::PAGE
        .within(MAX_WEB_SEARCH_URL_BYTES)
        .parse(raw)
        .ok()?;
    url.set_fragment(None);
    let retained = url
        .query_pairs()
        .filter(|(key, _)| {
            let lower = key.to_ascii_lowercase();
            !lower.starts_with("utm_")
                && !matches!(lower.as_str(), "fbclid" | "gclid" | "mc_cid" | "mc_eid")
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    if !retained.is_empty() {
        url.query_pairs_mut().extend_pairs(retained);
    }
    let rendered = url.to_string();
    (rendered.len() <= MAX_WEB_SEARCH_URL_BYTES).then_some(rendered)
}

pub(super) fn bounded_strings(
    values: Vec<String>,
    maximum: usize,
    maximum_bytes: usize,
) -> Vec<String> {
    values
        .into_iter()
        .map(|value| truncate_utf8(value.trim(), maximum_bytes))
        .filter(|value| !value.is_empty())
        .take(maximum)
        .collect()
}

fn truncate_utf8(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_string();
    }
    let boundary = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= maximum_bytes)
        .last()
        .unwrap_or(0);
    value[..boundary].to_string()
}

pub(super) fn empty_response(
    request: &WebSearchRequest,
    failures: Vec<WebSearchFailure>,
) -> WebSearchResponse {
    WebSearchResponse {
        query: request.query.clone(),
        providers: failures.iter().map(|failure| failure.provider).collect(),
        results: Vec::new(),
        answers: Vec::new(),
        suggestions: Vec::new(),
        partial_failures: failures,
        cache_hit: false,
    }
}
