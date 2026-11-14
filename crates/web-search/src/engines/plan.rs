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

// Derived from dsh-free-search 6ab5168 (MIT), routing and intent.js.

use super::{SearchOptions, WebSearchProvider as E};

pub(crate) const FALLBACK: &[E] = &[
    E::Exa,
    E::Tavily,
    E::Keenable,
    E::Firecrawl,
    E::Parallel,
    E::Perplexity,
    E::Serpbase,
    E::Serply,
    E::Outcrawl,
    E::Google,
    E::Agy,
    E::DeepseekTools,
    E::DeepseekOfficial,
    E::You,
    E::Baidu,
    E::Kimi,
    E::Aliyun,
    E::Doubao,
    E::Tinyfish,
    E::Jina,
    E::Bing,
    E::Anysearch,
    E::Duckduckgo,
    E::DdgLite,
    E::Searxng,
    E::Degoog,
    E::ZhihuGlobal,
    E::ZhihuSite,
];

impl SearchOptions {
    pub(crate) fn language_pool(&self, query: &str) -> Vec<E> {
        let cjk = query.chars().any(|c| matches!(c, '\u{4e00}'..='\u{9fff}' | '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7af}'));
        let pool = if cjk { &self.zh_pool } else { &self.en_pool };
        unique(pool.iter().copied().filter(|e| {
            self.enabled(*e) && (!e.explicit_only() || self.allowed_engines.contains(e))
        }))
    }

    pub(crate) fn route(&self, query: &str, time: bool) -> Vec<E> {
        let mut head = self.language_pool(query);
        let custom = !self.fallback_order.is_empty();
        let enabled = unique(
            self.fallback_order
                .iter()
                .chain(FALLBACK)
                .chain(self.allowed_engines.iter())
                .chain(head.iter())
                .copied(),
        )
        .into_iter()
        .filter(|e| self.enabled(*e) && (!e.explicit_only() || self.allowed_engines.contains(e)))
        .collect::<Vec<_>>();
        if custom {
            head.sort_by_key(|e| enabled.iter().position(|candidate| candidate == e));
        } else {
            let preference = intent_preference(query);
            head.sort_by_key(|e| {
                preference
                    .iter()
                    .position(|candidate| candidate == e)
                    .unwrap_or(usize::MAX)
            });
        }
        let mut ordered = if time {
            head.iter()
                .chain(&enabled)
                .filter(|e| e.supports_time())
                .copied()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        ordered.extend(head);
        ordered.extend(enabled);
        unique(ordered)
    }
}

fn unique(values: impl IntoIterator<Item = E>) -> Vec<E> {
    let mut out = Vec::new();
    for value in values {
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

fn intent_preference(query: &str) -> &'static [E] {
    let lower = query.to_lowercase();
    let rules: &[(&[&str], &[E])] = &[
        (
            &[
                "股票",
                "基金",
                "财报",
                "利率",
                "汇率",
                "贷款",
                "债券",
                "通胀",
                "市值",
                "ipo",
                "加密货币",
                "比特币",
                "stock",
                "equity",
                "fund",
                "earnings",
                "inflation",
                "interest rate",
                "forex",
                "lpr",
                "crypto",
                "bitcoin",
                "nasdaq",
                "nyse",
            ],
            &[
                E::Baidu,
                E::Bing,
                E::Anysearch,
                E::Exa,
                E::Tavily,
                E::Duckduckgo,
            ],
        ),
        (
            &[
                "论文",
                "文献",
                "学术",
                "引用",
                "期刊",
                "会议",
                "研究综述",
                "arxiv",
                "doi",
                "citation",
                "paper",
                "study",
                "pubmed",
                "scholar",
                "journal",
                "conference",
                "preprint",
            ],
            &[
                E::Exa,
                E::Tavily,
                E::Parallel,
                E::Bing,
                E::Anysearch,
                E::Duckduckgo,
            ],
        ),
        (
            &[
                "代码",
                "报错",
                "编译",
                "函数",
                "接口",
                "依赖",
                "仓库",
                "github",
                "gitlab",
                "npm",
                "pypi",
                "python",
                "javascript",
                "typescript",
                "rust",
                "golang",
                "java",
                "docker",
                "kubernetes",
                "stack trace",
                "exception",
                "regex",
                "api error",
            ],
            &[
                E::Bing,
                E::Duckduckgo,
                E::Exa,
                E::Tavily,
                E::Degoog,
                E::Anysearch,
            ],
        ),
        (
            &[
                "新闻",
                "最新",
                "今天",
                "昨天",
                "本周",
                "本月",
                "今年",
                "发布",
                "突发",
                "breaking",
                "today",
                "latest",
                "this week",
                "this month",
                "announced",
                "launch",
            ],
            &[E::Bing, E::Baidu, E::Anysearch, E::Exa, E::Duckduckgo],
        ),
        (
            &[
                "价格",
                "购买",
                "优惠",
                "折扣",
                "测评",
                "推荐",
                "哪款",
                "性价比",
                "price",
                "buy",
                "discount",
                "deal",
                "review",
                "best",
                "cheap",
            ],
            &[E::Baidu, E::Bing, E::Anysearch, E::Duckduckgo, E::Exa],
        ),
    ];
    rules
        .iter()
        .find(|(words, _)| words.iter().any(|word| lower.contains(word)))
        .map(|(_, order)| *order)
        .unwrap_or(&[])
}
