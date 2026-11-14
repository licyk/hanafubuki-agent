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

use crate::WebSearchProvider as E;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResearchOptions {
    pub primary_pool: Vec<E>,
    pub escalation_pool: Vec<E>,
    pub rerank_key: String,
    pub rerank_endpoint: String,
    pub decision_key: String,
    pub decision_endpoint: String,
}
impl Default for ResearchOptions {
    fn default() -> Self {
        Self {
            primary_pool: vec![
                E::Anysearch,
                E::Exa,
                E::Keenable,
                E::Bing,
                E::ZhihuGlobal,
                E::Parallel,
            ],
            escalation_pool: vec![E::Doubao, E::Baidu, E::Aliyun, E::Firecrawl, E::Kimi],
            rerank_key: String::new(),
            rerank_endpoint: "https://api.siliconflow.cn/v1".into(),
            decision_key: String::new(),
            decision_endpoint: String::new(),
        }
    }
}
impl std::fmt::Debug for ResearchOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResearchOptions")
            .field("primary_pool", &self.primary_pool)
            .field("escalation_pool", &self.escalation_pool)
            .finish_non_exhaustive()
    }
}
pub(crate) fn configured_or_environment(value: &str, name: &str) -> String {
    if value.trim().is_empty() {
        std::env::var(name).unwrap_or_default()
    } else {
        value.trim().into()
    }
}
