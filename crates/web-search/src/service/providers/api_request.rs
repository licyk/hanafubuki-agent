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

// Derived from dsh-free-search 6ab5168 (MIT), lib/engines REST requests.

use serde::Serialize;

#[derive(Serialize)]
pub(super) struct Query<'a> {
    pub query: &'a str,
    pub max_results: usize,
}
#[derive(Serialize)]
pub(super) struct LimitedQuery<'a> {
    pub query: &'a str,
    pub limit: usize,
}
#[derive(Serialize)]
pub(super) struct Tavily<'a> {
    pub query: &'a str,
    pub max_results: usize,
    pub search_depth: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_range: Option<&'static str>,
}
#[derive(Serialize)]
pub(super) struct Firecrawl<'a> {
    pub query: &'a str,
    pub limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tbs: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Exa<'a> {
    pub query: &'a str,
    #[serde(rename = "type")]
    pub search_type: &'static str,
    pub num_results: usize,
    pub contents: ExaContents,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_published_date: Option<String>,
}
#[derive(Serialize)]
pub(super) struct ExaContents {
    pub highlights: ExaHighlights,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExaHighlights {
    pub highlights_per_url: usize,
}
#[derive(Serialize)]
pub(super) struct Keenable<'a> {
    pub query: &'a str,
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_after: Option<String>,
}
#[derive(Serialize)]
pub(super) struct Parallel<'a> {
    pub objective: &'a str,
    pub search_queries: [&'a str; 1],
    pub mode: &'static str,
    pub advanced_settings: ParallelSettings,
}
#[derive(Serialize)]
pub(super) struct ParallelSettings {
    pub max_results: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_policy: Option<SourcePolicy>,
}
#[derive(Serialize)]
pub(super) struct SourcePolicy {
    pub after_date: String,
}
#[derive(Serialize)]
pub(super) struct Serpbase<'a> {
    pub q: &'a str,
    pub hl: &'a str,
    pub gl: &'a str,
    pub page: usize,
}
