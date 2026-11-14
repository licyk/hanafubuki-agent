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

mod documents;
mod fetch;
mod followup;

use super::{
    clusters::{self, Candidate},
    intent::{self, Intent},
    windows, DocumentKind, ResearchDocument, ResearchReport, SearchBudget, TextOrigin,
};
use crate::{WebSearchError, WebSearchProvider as E, WebSearchRequest, WebSearchService};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use std::sync::Arc;

struct Run<'a> {
    service: &'a WebSearchService,
    budgeted: WebSearchService,
    budget: Arc<SearchBudget>,
    cancellation: CancellationToken,
    report: ResearchReport,
    intent: Intent,
    depth: usize,
    candidates: Vec<Candidate>,
    bodies: Vec<Body>,
    candidate_used: usize,
    window_used: usize,
    evaluated: bool,
    token_remaining: usize,
}
struct Body {
    updated_at: Option<String>,
    candidate: Candidate,
    content: String,
    fetched_at: i64,
    tier: String,
    cache: bool,
    truncated: bool,
    round: u8,
    selected: Option<Vec<windows::Window>>,
    folded_into: Option<String>,
}

impl WebSearchService {
    pub async fn research(
        &self,
        question: &str,
        depth: usize,
        cancellation: CancellationToken,
    ) -> Result<ResearchReport, WebSearchError> {
        if cancellation.is_cancelled() {
            return Err(WebSearchError::Cancelled);
        }
        let depth = depth.clamp(1, 5);
        let budget = Arc::new(SearchBudget::new(depth));
        let mut budgeted = self.clone();
        budgeted.search_budget = Some(budget.clone());
        let child = CancellationToken::default();
        let mut run = Run {
            service: self,
            budgeted,
            budget: budget.clone(),
            cancellation: child.clone(),
            report: ResearchReport::new(question),
            intent: intent::judge(question),
            depth,
            candidates: Vec::new(),
            bodies: Vec::new(),
            candidate_used: 0,
            window_used: 0,
            evaluated: false,
            token_remaining: depth * 1500,
        };
        tokio::select! {
            result=run.execute()=>if let Err(error)=result {run.report.notes.push(format!("search failed: {error}"));},
            _=tokio::time::sleep_until(tokio::time::Instant::from_std(budget.deadline))=>{child.cancel();run.recover_partial();run.report.notes.push("deadline_hit: 90s elapsed; queued work not started".into());},
            _=super::super::http::wait_cancelled(&cancellation)=>{child.cancel();return Err(WebSearchError::Cancelled);},
        }
        run.render_pending();
        run.add_snippets();
        if budget.used() >= 12 {
            run.report
                .notes
                .push("budget_exhausted: 12 search requests including retries and fallback".into());
        }
        if budget.fetch_exhausted() {
            run.report
                .notes
                .push("budget_exhausted: fetch attempts".into());
        }
        if run.bodies.is_empty() {
            run.report
                .notes
                .push("notEvaluated: all fetches failed; snippets only".into());
        }
        run.report.assess();
        Ok(run.report)
    }
}
impl Run<'_> {
    async fn execute(&mut self) -> Result<(), WebSearchError> {
        self.service
            .decide_intent(&self.report.question, &mut self.intent, &self.cancellation)
            .await;
        self.report.notes.push(format!(
            "intent({}): task={} comparison={} freshness={:?} language={} breadth={}",
            self.intent.source,
            self.intent.task,
            self.intent.comparison,
            self.intent.freshness,
            self.intent.language,
            self.intent.breadth
        ));
        let query = self.report.question.clone();
        let pool = self
            .service
            .inner
            .config
            .search
            .research
            .primary_pool
            .clone();
        let candidates = self.search_round(&query, pool, 1).await?;
        self.process_round(candidates, 1).await;
        self.render_pending();
        self.add_snippets();
        self.followup().await;
        Ok(())
    }
    async fn search_round(
        &mut self,
        query: &str,
        engines: Vec<E>,
        round: u8,
    ) -> Result<Vec<Candidate>, WebSearchError> {
        self.budget
            .round
            .store(round as usize, std::sync::atomic::Ordering::Relaxed);
        let request = WebSearchRequest {
            query: query.into(),
            maximum_results: 12,
            provider: E::Multi,
            engines,
            category: Default::default(),
            time_range: None,
            region: None,
        };
        let response = self
            .budgeted
            .search(request, self.cancellation.clone())
            .await?;
        for engine in response.providers {
            if !self.report.engines.contains(&engine) {
                self.report.engines.push(engine);
            }
        }
        for failure in response.partial_failures {
            self.report.notes.push(format!(
                "search engine {:?} failed: {:?}",
                failure.provider, failure.code
            ));
        }
        let mut candidates = clusters::build(&response.results);
        for candidate in &mut candidates {
            candidate.round = round;
        }
        candidates.retain(|c| !self.candidates.iter().any(|old| old.key == c.key));
        for c in &candidates {
            self.report.sources.push(c.source.clone());
        }
        self.candidates.extend(candidates.iter().cloned());
        Ok(candidates)
    }
    async fn process_round(&mut self, mut candidates: Vec<Candidate>, round: u8) {
        candidates.truncate(12);
        if let Some(vectors) = self
            .service
            .embeddings(
                &candidates
                    .iter()
                    .map(|c| format!("{}\n{}", c.source.title, c.source.snippet))
                    .collect::<Vec<_>>(),
                &self.cancellation,
            )
            .await
        {
            clusters::semantic_tags(&mut candidates, &vectors);
        }
        let slots = 12usize
            .saturating_sub(self.candidate_used)
            .min(candidates.len());
        let texts: Vec<_> = candidates
            .iter()
            .take(slots)
            .map(|c| format!("{}\n{}", c.source.title, c.source.snippet))
            .collect();
        self.candidate_used += slots;
        if let Some(scores) = self
            .service
            .rerank(&self.report.question, &texts, &self.cancellation)
            .await
        {
            self.evaluated = true;
            let mut scored: Vec<_> = candidates
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    let score = scores.get(i).copied().unwrap_or(0.0)
                        - if c.representative { 0.0 } else { 0.15 };
                    (i, c, score)
                })
                .collect();
            scored.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
            candidates = scored.into_iter().map(|(_, c, _)| c).collect();
        } else {
            self.report
                .notes
                .push("notEvaluated: stage-A rerank unavailable; rule ordering used".into());
            candidates.sort_by_key(|c| !c.representative);
        }
        self.fetch_candidates(candidates, round).await;
        self.select_windows(round).await;
    }
    fn recover_partial(&mut self) {
        for (round, response) in self.budget.partial.lock().iter() {
            for mut candidate in clusters::build(&response.results) {
                if !self.candidates.iter().any(|old| old.key == candidate.key)
                    && !self.service.blocked(&candidate.source.url)
                {
                    candidate.round = *round;
                    self.report.sources.push(candidate.source.clone());
                    self.candidates.push(candidate);
                }
            }
            for engine in &response.providers {
                if !self.report.engines.contains(engine) {
                    self.report.engines.push(*engine);
                }
            }
        }
    }
    fn add_snippets(&mut self) {
        for candidate in &self.candidates {
            if self
                .report
                .documents
                .iter()
                .any(|d| d.url == candidate.source.url)
                || self.bodies.iter().any(|b| b.candidate.key == candidate.key)
            {
                continue;
            }
            self.report.documents.push(ResearchDocument {
                url: candidate.source.url.clone(),
                title: candidate.source.title.clone(),
                kind: DocumentKind::SnippetOnly,
                text_origin: if candidate.source.snippet.starts_with("[provider-summary]") {
                    TextOrigin::ProviderSummary
                } else {
                    TextOrigin::Original
                },
                engine_hits: candidate.source.engines.clone(),
                published: candidate.source.published_at.clone(),
                updated_at: None,
                fetched_at: chrono::Utc::now().to_rfc3339(),
                round: candidate.round,
                cluster: candidate.cluster.clone(),
                independence: "unknown".into(),
                tier: "snippet".into(),
                cache: false,
                truncated: false,
                content: candidate.source.snippet.clone(),
            });
        }
    }
}
