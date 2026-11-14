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

use crate::{EvidenceLevel, WebSearchProvider, WebSearchResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchDocument {
    pub url: String,
    pub title: String,
    pub kind: DocumentKind,
    pub text_origin: TextOrigin,
    pub engine_hits: Vec<String>,
    pub published: Option<String>,
    pub updated_at: Option<String>,
    pub fetched_at: String,
    pub round: u8,
    pub cluster: Option<String>,
    pub independence: String,
    pub tier: String,
    pub cache: bool,
    pub truncated: bool,
    pub content: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DocumentKind {
    Body,
    SnippetOnly,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TextOrigin {
    Original,
    ProviderSummary,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReport {
    pub question: String,
    pub engines: Vec<WebSearchProvider>,
    pub sources: Vec<WebSearchResult>,
    pub documents: Vec<ResearchDocument>,
    pub notes: Vec<String>,
    pub estimated_tokens: usize,
    pub quality: EvidenceQuality,
    pub untrusted_external_content: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceQuality {
    pub level: EvidenceLevel,
    pub cap: EvidenceLevel,
    pub sources: usize,
    pub domains: usize,
    pub dated: usize,
    pub fetched: usize,
    pub note: String,
}
impl ResearchReport {
    pub(crate) fn new(question: &str) -> Self {
        Self {
            question: question.into(),
            engines: Vec::new(),
            sources: Vec::new(),
            documents: Vec::new(),
            notes: Vec::new(),
            estimated_tokens: 0,
            quality: EvidenceQuality {
                level: EvidenceLevel::Low,
                cap: EvidenceLevel::Low,
                sources: 0,
                domains: 0,
                dated: 0,
                fetched: 0,
                note: "insufficient evidence; report gaps rather than conclusions".into(),
            },
            untrusted_external_content: true,
        }
    }
    pub(crate) fn assess(&mut self) {
        let domains = self
            .sources
            .iter()
            .filter_map(|s| url::Url::parse(&s.url).ok())
            .filter_map(|u| {
                u.host_str()
                    .map(|h| h.trim_start_matches("www.").to_lowercase())
            })
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let dated = self
            .sources
            .iter()
            .filter(|s| s.published_at.is_some())
            .count();
        let fetched = self
            .documents
            .iter()
            .filter(|d| d.kind == DocumentKind::Body)
            .count();
        let degraded = self.notes.iter().any(|s| {
            [
                "search failed",
                "budget_exhausted",
                "all fetches failed",
                "notEvaluated",
                "deadline_hit",
            ]
            .iter()
            .any(|n| s.contains(n))
        });
        let level = if degraded {
            EvidenceLevel::Low
        } else if self.sources.len() >= 8 && domains >= 3 && fetched >= 3 && dated >= 2 {
            EvidenceLevel::High
        } else if self.sources.len() >= 3 && domains >= 2 && fetched >= 1 {
            EvidenceLevel::Medium
        } else {
            EvidenceLevel::Low
        };
        self.quality=EvidenceQuality {level,cap:level,sources:self.sources.len(),domains,dated,fetched,note:match level {EvidenceLevel::High=>"multiple sources and fetched bodies; verify claims against the cited evidence",EvidenceLevel::Medium=>"thin evidence: verify key claims before concluding",EvidenceLevel::Low=>"insufficient evidence: do not state conclusions as fact; fetch more sources or report the gap"}.into()};
        self.estimated_tokens = self
            .sources
            .iter()
            .map(|s| super::windows::estimate_tokens(&format!("{}{}", s.title, s.snippet)))
            .sum::<usize>()
            + self
                .documents
                .iter()
                .map(|d| super::windows::estimate_tokens(&d.content))
                .sum::<usize>();
    }
}
