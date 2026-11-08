use std::sync::Arc;

use hanafubuki_agent_contracts::{Register, ToolRegistry};
use hanafubuki_agent_web_search::{
    WebSearchConfig, WebSearchError, WebSearchProviderPreference, WebSearchService,
};
use kardia_core::{Context, Inject, PluginDef, Service};

use crate::tools::{FetchWebPageTool, WebSearchTool};
use crate::WebPluginConfig;

/// Registers `web_search` and `fetch_web_page` over one `WebSearchService`
/// built from the plugin's configuration.
///
/// Search endpoints that cannot be used (SearXNG selected without an
/// address, an address that does not parse) cost only `web_search`:
/// the plugin starts, `web_search` is left out of every step and says why if
/// it is called, and `fetch_web_page` works. Any other configuration fault
/// fails the plugin.
pub fn web_plugin() -> PluginDef {
    PluginDef::typed(|ctx: Context, config: Arc<WebPluginConfig>| async move {
        let tools = ctx.get::<ToolRegistry>()?;
        let (service_config, search_unavailable) = without_unusable_search(config.service_config());
        if let Some(reason) = &search_unavailable {
            log::warn!("{reason}; web_search is unavailable and fetch_web_page still works");
        }
        let service = WebSearchService::new(service_config)
            .map_err(|error| kardia_core::Error::msg(format!("web search: {error}")))?;
        tools.register(Arc::new(
            WebSearchTool::new(service.clone(), search_unavailable)
                .map_err(kardia_core::Error::new)?,
        ))?;
        tools.register(Arc::new(
            FetchWebPageTool::new(service).map_err(kardia_core::Error::new)?,
        ))?;
        Ok(())
    })
    .name("web")
    .inject(Inject::new().with(ToolRegistry::NAME))
}

/// The configuration with search endpoints that cannot be used removed, and
/// why search is then unavailable. The settings validated the address, so
/// this is the service's own reading of it.
fn without_unusable_search(mut config: WebSearchConfig) -> (WebSearchConfig, Option<String>) {
    let Some(error) = config.search_endpoint_error() else {
        return (config, None);
    };
    let reason = match error {
        WebSearchError::InvalidConfiguration(message) => message.to_string(),
        other => other.to_string(),
    };
    config.searxng_base_url = None;
    config.provider_preference = WebSearchProviderPreference::Auto;
    (
        config,
        Some(format!("web search is not configured: {reason}")),
    )
}

#[cfg(test)]
mod tests;
