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

use crate::{
    FailureClass, WebSearchError, WebSearchProvider as E, WebSearchRequest, WebSearchService,
};
use futures_util::{stream, StreamExt};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineCheck {
    pub engine: E,
    pub status: &'static str,
    pub results: usize,
    pub providers: Vec<E>,
    pub sample_title: Option<String>,
    pub sample_url: Option<String>,
    pub error: Option<String>,
    pub failure_class: Option<FailureClass>,
}
impl WebSearchService {
    pub async fn test_engines(
        &self,
        query: &str,
        engines: Vec<E>,
        cancellation: CancellationToken,
    ) -> Result<Vec<EngineCheck>, WebSearchError> {
        let engines = if engines.is_empty() {
            self.inner
                .config
                .search
                .route(query, false)
                .into_iter()
                .filter(|e| !e.explicit_only())
                .collect()
        } else {
            engines
        };
        let mut running = stream::iter(engines.into_iter().map(|engine| {
            let cancellation = cancellation.clone();
            async move {
                let mut check = EngineCheck {
                    engine,
                    status: "disabled",
                    results: 0,
                    providers: Vec::new(),
                    sample_title: None,
                    sample_url: None,
                    error: None,
                    failure_class: None,
                };
                if !matches!(engine, E::Auto | E::Multi)
                    && !self.inner.config.search.enabled(engine)
                {
                    return Ok(check);
                }
                let request = WebSearchRequest {
                    engines: Vec::new(),
                    query: query.into(),
                    maximum_results: 2,
                    provider: engine,
                    category: Default::default(),
                    time_range: None,
                    region: None,
                };
                let result = if matches!(engine, E::Auto | E::Multi) {
                    self.execute(&request, &cancellation).await
                } else {
                    self.fetch_provider(engine, &request, &cancellation).await
                };
                match result {
                    Ok(response) => {
                        check.status = if response.results.is_empty() {
                            "empty"
                        } else {
                            "ok"
                        };
                        check.results = response.results.len();
                        check.providers = response.providers;
                        if let Some(first) = response.results.first() {
                            check.sample_title = Some(first.title.clone());
                            check.sample_url = Some(first.url.clone());
                        }
                    }
                    Err(WebSearchError::Cancelled) => return Err(WebSearchError::Cancelled),
                    Err(error) => {
                        check.status = "failed";
                        check.failure_class = Some(error.class());
                        check.error = Some(error.to_string());
                    }
                }
                Ok(check)
            }
        }))
        .buffered(2);
        let mut results = Vec::new();
        while let Some(result) = running.next().await {
            results.push(result?);
        }
        Ok(results)
    }
}
