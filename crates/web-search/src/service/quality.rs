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

//! Heuristic source warnings from dsh-free-search; never a credibility score.
use crate::WebSearchResponse;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchUncertainty {
    pub level: EvidenceLevel,
    pub notes: Vec<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceLevel {
    Low,
    Medium,
    High,
}

pub(super) fn assess(
    response: &WebSearchResponse,
    time_filter: bool,
    chinese: bool,
) -> SearchUncertainty {
    let domains: BTreeSet<_> = response
        .results
        .iter()
        .filter_map(|r| url::Url::parse(&r.url).ok())
        .filter_map(|url| {
            url.host_str()
                .map(|h| h.trim_start_matches("www.").to_lowercase())
        })
        .collect();
    let mut level = EvidenceLevel::High;
    let mut notes = Vec::new();
    let mut note = |en: &str, zh: &str| notes.push(if chinese { zh } else { en }.to_string());
    match response.results.len() {
        0 => {
            level = EvidenceLevel::Low;
            note("no results returned", "没有返回结果");
        }
        1 => {
            level = EvidenceLevel::Low;
            note("only one source", "只有一个来源");
        }
        _ if domains.len() == 1 => {
            level = EvidenceLevel::Medium;
            note(
                "all results come from a single site",
                "所有结果来自同一站点",
            );
        }
        2 => {
            level = EvidenceLevel::Medium;
            note("only two sources", "只有两个来源");
        }
        _ => {}
    }
    if time_filter && response.results.iter().all(|r| r.published_at.is_none()) {
        if level == EvidenceLevel::High {
            level = EvidenceLevel::Medium;
        }
        note(
            "time filter applied but results carry no publish dates",
            "带时间过滤但结果没有发布时间",
        );
    }
    if !response.partial_failures.is_empty() {
        level = EvidenceLevel::Low;
        note(
            "some engines failed; the result may be incomplete",
            "部分引擎失败，结果可能不完整",
        );
    }
    if domains.iter().any(|host| {
        host.split('.').any(|part| {
            [
                "zhihu",
                "sohu",
                "sina",
                "163",
                "qq",
                "toutiao",
                "baijiahao",
                "csdn",
                "jianshu",
                "medium",
                "substack",
                "reddit",
                "quora",
                "stackexchange",
            ]
            .contains(&part)
        })
    }) {
        if level == EvidenceLevel::High {
            level = EvidenceLevel::Medium;
        }
        note(
            "includes aggregator content; verify against original sources",
            "包含聚合站内容，建议二次核对",
        );
    }
    SearchUncertainty { level, notes }
}
