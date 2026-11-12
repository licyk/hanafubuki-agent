use hanafubuki_agent_web_search::{WebSearchConfig, WebSearchProviderPreference, WebSearchProxy};
use serde::{Deserialize, Serialize};

/// The web plugin's configuration, derived by the host from the
/// application's web search and proxy settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebPluginConfig {
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
