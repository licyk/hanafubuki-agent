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

use std::sync::Arc;

use hanafubuki_agent_contracts::{Register, ToolPresenterRegistry, ToolRegistry};
use hanafubuki_agent_web_search::{
    WebSearchConfig, WebSearchError, WebSearchProviderPreference, WebSearchService,
};
use kardia_core::{Context, Inject, PluginDef, Service};

use crate::presenter::WebPresenter;
use crate::tools::{AuxiliaryWebTool, FetchWebPageTool, WebSearchTool, WebToolKind};
use crate::WebPluginConfig;

/// Registers search, research, public-data and extraction tools over one `WebSearchService`
/// built from the plugin's configuration, and their presenter.
///
/// Search endpoints that cannot be used (SearXNG selected without an
/// address, an address that does not parse) cost only `web_search`:
/// the plugin starts, `web_search` is left out of every step and says why if
/// it is called, and `fetch_web_page` works. Any other configuration fault
/// fails the plugin.
pub fn web_plugin(
    executor: Option<hanafubuki_agent_process::executor::ExecutorManager>,
    state_directory: Option<std::path::PathBuf>,
) -> PluginDef {
    let runner = executor.map(|executor| {
        Arc::new(crate::local::OwnedWebRunner { executor })
            as Arc<dyn hanafubuki_agent_web_search::LocalWebRunner>
    });
    PluginDef::typed(move |ctx: Context, config: Arc<WebPluginConfig>| {
        let runner = runner.clone();
        let state_directory = state_directory.clone();
        async move {
            let tools = ctx.get::<ToolRegistry>()?;
            let presenters = ctx.get::<ToolPresenterRegistry>()?;
            let (mut service_config, search_unavailable) =
                without_unusable_search(config.service_config());
            if let Some(reason) = &search_unavailable {
                log::warn!("{reason}; web_search is unavailable and fetch_web_page still works");
            }
            service_config.state_directory = state_directory;
            let service = WebSearchService::new(service_config)
                .map_err(|error| kardia_core::Error::msg(format!("web search: {error}")))?;
            let service = if config.search.local_helpers_enabled {
                service.with_local_runner(runner)
            } else {
                service
            };
            if !config
                .search
                .disabled_tools
                .iter()
                .any(|name| name == "web_search")
            {
                tools.register(Arc::new(
                    WebSearchTool::new(service.clone(), search_unavailable.clone())
                        .map_err(kardia_core::Error::new)?,
                ))?;
            }
            for name in ["advanced_search", "multi_search"] {
                if !config
                    .search
                    .disabled_tools
                    .iter()
                    .any(|disabled| disabled == name)
                {
                    tools.register(Arc::new(
                        WebSearchTool::named(name, service.clone(), search_unavailable.clone())
                            .map_err(kardia_core::Error::new)?,
                    ))?;
                }
            }
            for kind in WebToolKind::ALL {
                if !config
                    .search
                    .disabled_tools
                    .iter()
                    .any(|name| name == kind.name())
                {
                    tools.register(Arc::new(
                        AuxiliaryWebTool::new(kind, service.clone())
                            .map_err(kardia_core::Error::new)?,
                    ))?;
                }
            }
            if !config
                .search
                .disabled_tools
                .iter()
                .any(|name| name == "fetch_web_page")
            {
                tools.register(Arc::new(
                    FetchWebPageTool::new(service).map_err(kardia_core::Error::new)?,
                ))?;
            }
            presenters.register(Arc::new(WebPresenter))?;
            Ok(())
        }
    })
    .name("web")
    .inject(
        Inject::new()
            .with(ToolRegistry::NAME)
            .with(ToolPresenterRegistry::NAME),
    )
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
