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

use hanafubuki_agent_web_search::{WebSearchConfig, WebSearchProviderPreference, WebSearchProxy};
use serde::{Deserialize, Serialize};

/// The web plugin's configuration, derived by the host from the
/// application's web search and proxy settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebPluginConfig {
    pub search: hanafubuki_agent_web_search::SearchOptions,
    pub provider: WebProviderSetting,
    pub searxng_base_url: Option<String>,
    pub proxy: WebProxySetting,
    pub jina_reader_enabled: bool,
    /// Through a proxy URL, a page's name this machine cannot resolve is
    /// left to the proxy.
    pub resolve_through_proxy: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebProviderSetting {
    #[default]
    Auto,
    Duckduckgo,
    Searxng,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode", content = "address")]
pub enum WebProxySetting {
    #[default]
    System,
    Disabled,
    Explicit(String),
}

impl WebPluginConfig {
    pub(crate) fn service_config(&self) -> WebSearchConfig {
        let mut config = WebSearchConfig::default();
        config.search = self.search.clone();
        config.cache_ttl =
            std::time::Duration::from_secs(self.search.cache_ttl_minutes.min(5) * 60);
        config.provider_preference = match self.provider {
            WebProviderSetting::Auto => WebSearchProviderPreference::Auto,
            WebProviderSetting::Duckduckgo => WebSearchProviderPreference::Duckduckgo,
            WebProviderSetting::Searxng => WebSearchProviderPreference::Searxng,
        };
        config.searxng_base_url = self.searxng_base_url.clone();
        config.proxy = match &self.proxy {
            WebProxySetting::System => WebSearchProxy::System,
            WebProxySetting::Disabled => WebSearchProxy::Disabled,
            WebProxySetting::Explicit(address) => WebSearchProxy::Explicit(address.clone()),
        };
        config.jina_reader_enabled = self.jina_reader_enabled;
        config.resolve_through_proxy = self.resolve_through_proxy;
        config
    }
}
