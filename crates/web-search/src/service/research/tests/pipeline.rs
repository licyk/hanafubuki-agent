// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use super::fixtures;
use crate::{
    DocumentKind, EngineConfig, WebSearchConfig, WebSearchProvider as E, WebSearchProxy,
    WebSearchService,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

#[tokio::test]
async fn research_keeps_first_round_and_uses_persistent_documents_on_repeat() {
    let server = fixtures::server(false).await;
    let directory = std::env::temp_dir().join(format!(
        "hana-research-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut config = WebSearchConfig {
        proxy: WebSearchProxy::Disabled,
        minimum_request_interval: Duration::ZERO,
        state_directory: Some(directory.clone()),
        ..Default::default()
    };
    config.test_allowed_private_origins.push(server.url.clone());
    config.search.allowed_engines = vec![E::Tavily];
    config.search.engines.insert(
        E::Tavily,
        EngineConfig {
            endpoint: Some(format!("{}/search", server.url)),
            api_keys: vec!["fixture".into()],
            ..Default::default()
        },
    );
    config.search.research.primary_pool = vec![E::Tavily];
    config.search.research.escalation_pool = vec![E::Tavily];
    config.search.research.rerank_endpoint = server.url.clone();
    config.search.research.rerank_key = "fixture".into();
    config.search.research.decision_endpoint = format!("{}/decision", server.url);
    config.search.research.decision_key = "fixture".into();
    let service = WebSearchService::new(config.clone()).unwrap();
    let report = tokio::time::timeout(
        Duration::from_secs(5),
        service.research("Compare Alpha vs Beta", 1, CancellationToken::default()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        server.searches.load(Ordering::Relaxed),
        2,
        "{:?}",
        report.notes
    );
    assert_eq!(
        server.pages.load(Ordering::Relaxed),
        2,
        "{:?}",
        report.notes
    );
    assert_eq!(
        report
            .documents
            .iter()
            .filter(|d| d.kind == DocumentKind::Body)
            .count(),
        2
    );
    assert!(report
        .documents
        .iter()
        .any(|d| d.round == 1 && d.content.contains("Compare Alpha")));
    assert!(report
        .documents
        .iter()
        .any(|d| d.round == 2 && d.content.contains("Beta")));
    assert!(
        report.notes.iter().any(|n| n.contains("gap_filled=true")),
        "{:?}",
        report.notes
    );
    assert_eq!(service.cache_search("Alpha", 8).len(), 1);
    let fetched = report.documents[0].fetched_at.clone();
    drop(service);
    // A new service reads the same SQLite cache. Use the source query's cached
    // URL directly so this assertion does not depend on a third search fixture.
    let service = WebSearchService::new(config).unwrap();
    let cached = service
        .inner
        .documents
        .get(&format!("{}/page/alpha", server.url), 86400)
        .unwrap();
    assert_eq!(
        chrono::DateTime::from_timestamp(cached.fetched_at, 0)
            .unwrap()
            .to_rfc3339(),
        fetched
    );
    assert!(cached.content.contains("Compare Alpha"));
    drop(service);
    server.task.abort();
    let _ = server.task.await;
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn physical_requests_including_key_rotation_stop_at_research_cap() {
    let server = fixtures::server(true).await;
    let mut config = WebSearchConfig {
        proxy: WebSearchProxy::Disabled,
        minimum_request_interval: Duration::ZERO,
        ..Default::default()
    };
    config.search.allowed_engines = vec![E::Exa];
    config.search.engines.insert(
        E::Exa,
        EngineConfig {
            endpoint: Some(server.url.clone()),
            api_keys: (0..15).map(|i| format!("fixture-{i}")).collect(),
            ..Default::default()
        },
    );
    let mut service = WebSearchService::new(config).unwrap();
    let budget = Arc::new(super::super::SearchBudget::new(1));
    service.search_budget = Some(budget.clone());
    let request = crate::WebSearchRequest {
        query: "test budget".into(),
        maximum_results: 2,
        provider: E::Exa,
        engines: Vec::new(),
        category: Default::default(),
        time_range: None,
        region: None,
    };
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        service.search(request, CancellationToken::default()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(response.results.is_empty());
    assert_eq!(server.searches.load(Ordering::Relaxed), 12);
    assert_eq!(budget.used(), 12);
    server.task.abort();
    let _ = server.task.await;
}
