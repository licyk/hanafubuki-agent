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

// Derived from dsh-free-search 6ab5168 (MIT), configuration and multi_policy.js.

use super::WebSearchProvider;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EngineConfig {
    pub api_keys: Vec<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub instances: Vec<String>,
    pub google_cse_id: String,
    pub metaso_scope: String,
    pub yandex_folder_id: String,
    pub yandex_search_type: String,
    pub yuanbao_mode: u8,
    pub yuanbao_site: String,
}

impl std::fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineConfig")
            .field("api_keys", &"[redacted]")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl EngineConfig {
    pub(crate) fn keys(&self, engine: WebSearchProvider) -> Vec<String> {
        let explicit = self.api_keys.join("\n");
        let raw = if explicit.trim().is_empty() {
            engine
                .key_environment()
                .iter()
                .filter_map(|name| std::env::var(name).ok())
                .find(|value| !value.trim().is_empty())
                .unwrap_or_default()
        } else {
            explicit
        };
        let mut keys = Vec::new();
        for key in raw
            .split(['\n', ',', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !keys.iter().any(|old| old == key) {
                keys.push(key.to_string());
            }
        }
        keys
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchOptions {
    pub local_helpers_enabled: bool,
    pub cache_ttl_minutes: u64,
    pub region: Option<String>,
    pub platforms: Vec<crate::Platform>,
    pub prompt_language: String,
    pub fetch: crate::FetchOptions,
    pub research: crate::ResearchOptions,
    pub provider: WebSearchProvider,
    pub engines: BTreeMap<WebSearchProvider, EngineConfig>,
    pub allowed_engines: Vec<WebSearchProvider>,
    pub disabled_engines: Vec<WebSearchProvider>,
    pub fallback_order: Vec<WebSearchProvider>,
    pub fallback_on: Vec<FailureClass>,
    pub blocked_domains: Vec<String>,
    pub zh_pool: Vec<WebSearchProvider>,
    pub en_pool: Vec<WebSearchProvider>,
    pub language: String,
    pub bing_market: String,
    pub safe_search: SafeSearch,
    pub multi_max_engines: usize,
    pub multi_min_sources: usize,
    pub multi_max_results: usize,
    pub cache_enabled: bool,
    pub disabled_tools: Vec<String>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        use WebSearchProvider as E;
        Self {
            local_helpers_enabled: false,
            cache_ttl_minutes: 5,
            region: None,
            platforms: crate::Platform::ALL.to_vec(),
            prompt_language: "en".into(),
            research: crate::ResearchOptions::default(),
            fetch: crate::FetchOptions::default(),
            provider: E::Auto,
            engines: BTreeMap::new(),
            allowed_engines: Vec::new(),
            disabled_engines: Vec::new(),
            fallback_order: Vec::new(),
            fallback_on: vec![
                FailureClass::Region,
                FailureClass::Quota,
                FailureClass::Auth,
                FailureClass::BotWall,
                FailureClass::Transient,
                FailureClass::InvalidResponse,
                FailureClass::Unknown,
            ],
            blocked_domains: Vec::new(),
            zh_pool: vec![E::Bing, E::Baidu, E::Aliyun, E::Anysearch],
            en_pool: vec![E::Bing, E::Exa, E::Tavily],
            language: "zh".into(),
            bing_market: String::new(),
            safe_search: SafeSearch::Off,
            multi_max_engines: 0,
            multi_min_sources: 1,
            multi_max_results: 20,
            cache_enabled: true,
            disabled_tools: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureClass {
    Region,
    Quota,
    Auth,
    BotWall,
    Transient,
    InvalidResponse,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeSearch {
    #[default]
    Off,
    Moderate,
    Strict,
}

impl SearchOptions {
    pub fn enabled(&self, engine: WebSearchProvider) -> bool {
        WebSearchProvider::ALL.contains(&engine)
            && !self.disabled_engines.contains(&engine)
            && (self.allowed_engines.is_empty() || self.allowed_engines.contains(&engine))
    }
}
