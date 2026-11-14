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

//! The web plugin registers its tools while it runs and removes them when it
//! stops or is reconfigured.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentMemoryContextEffect, RegistryService, ToolAvailability, ToolEffect, ToolPlanContext,
    ToolPresenterRegistry, ToolRegistry,
};
use hanafubuki_agent_plugin_web::{
    web_plugin, WebPluginConfig, WebProviderSetting, WebProxySetting,
};
use kardia_core::{Context, FiberState, PluginDef, Service};
use serde_json::Value;

async fn root_with_tools() -> (Context, Arc<ToolRegistry>) {
    let tools = Arc::new(ToolRegistry::default());
    let provided = tools.clone();
    let root = Context::new();
    let registry = PluginDef::new(move |ctx: Context, _| {
        let tools = provided.clone();
        async move {
            ctx.provide_arc(tools)?;
            ctx.provide(ToolPresenterRegistry::default())?;
            Ok(())
        }
    })
    .name("tools")
    .provides([ToolRegistry::NAME, ToolPresenterRegistry::NAME]);
    root.plugin_def(&registry, Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    (root, tools)
}

fn names(tools: &ToolRegistry) -> Vec<String> {
    let mut names = tools
        .registry()
        .snapshot()
        .items()
        .map(|tool| tool.descriptor().name.qualified())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[tokio::test]
async fn registers_web_tools_and_removes_them_when_stopped() {
    let (root, tools) = root_with_tools().await;
    let fiber = root
        .plugin_def(&web_plugin(None, None), Value::Null)
        .unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);
    assert_eq!(
        names(&tools),
        [
            "advanced_search",
            "cache_search",
            "domain_search",
            "fetch_web_page",
            "free_search_test",
            "multi_search",
            "platform_search",
            "research",
            "video_search",
            "weather",
            "web_search"
        ]
    );
    for tool in tools.registry().snapshot().items() {
        let descriptor = tool.descriptor();
        assert_eq!(descriptor.effect, ToolEffect::ReadOnly);
        assert_eq!(
            descriptor.memory_effect,
            AgentMemoryContextEffect::PollutesContribution
        );
    }

    let config = WebPluginConfig {
        provider: WebProviderSetting::Duckduckgo,
        ..WebPluginConfig::default()
    };
    fiber
        .update(serde_json::to_value(config).unwrap())
        .unwrap()
        .await
        .unwrap();
    assert_eq!(fiber.state(), FiberState::Active);
    assert_eq!(
        names(&tools),
        [
            "advanced_search",
            "cache_search",
            "domain_search",
            "fetch_web_page",
            "free_search_test",
            "multi_search",
            "platform_search",
            "research",
            "video_search",
            "weather",
            "web_search"
        ]
    );

    fiber.dispose().await.unwrap();
    assert!(names(&tools).is_empty());
    root.fiber().dispose().await.unwrap();
}

fn availability(tools: &ToolRegistry, name: &str) -> ToolAvailability {
    tools
        .registry()
        .snapshot()
        .items()
        .find(|tool| tool.descriptor().name.qualified() == name)
        .expect("the tool is registered")
        .availability(&ToolPlanContext::default())
}

#[tokio::test]
async fn an_unusable_searxng_endpoint_costs_only_web_search() {
    for config in [
        WebPluginConfig {
            provider: WebProviderSetting::Searxng,
            searxng_base_url: Some("not a url".to_string()),
            ..WebPluginConfig::default()
        },
        WebPluginConfig {
            provider: WebProviderSetting::Searxng,
            searxng_base_url: None,
            ..WebPluginConfig::default()
        },
    ] {
        let (root, tools) = root_with_tools().await;
        let fiber = root
            .plugin_def(
                &web_plugin(None, None),
                serde_json::to_value(&config).unwrap(),
            )
            .unwrap();
        fiber.settled().await.unwrap();
        assert_eq!(fiber.state(), FiberState::Active, "{config:?}");
        assert_eq!(
            names(&tools),
            [
                "advanced_search",
                "cache_search",
                "domain_search",
                "fetch_web_page",
                "free_search_test",
                "multi_search",
                "platform_search",
                "research",
                "video_search",
                "weather",
                "web_search"
            ]
        );
        assert_eq!(
            availability(&tools, "web_search"),
            ToolAvailability::Omitted
        );
        assert_eq!(
            availability(&tools, "fetch_web_page"),
            ToolAvailability::Function
        );

        // A configuration that fixes the endpoint brings search back.
        fiber
            .update(serde_json::to_value(WebPluginConfig::default()).unwrap())
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fiber.state(), FiberState::Active);
        assert_eq!(
            availability(&tools, "web_search"),
            ToolAvailability::Function
        );
        root.fiber().dispose().await.unwrap();
    }
}

#[tokio::test]
async fn a_configuration_fault_outside_search_still_fails_the_plugin() {
    let (root, tools) = root_with_tools().await;
    let config = WebPluginConfig {
        proxy: WebProxySetting::Explicit("not a proxy".to_string()),
        ..WebPluginConfig::default()
    };
    let fiber = root
        .plugin_def(
            &web_plugin(None, None),
            serde_json::to_value(config).unwrap(),
        )
        .unwrap();
    let _ = fiber.settled().await;
    assert_eq!(fiber.state(), FiberState::Failed);
    assert!(names(&tools).is_empty());
    root.fiber().dispose().await.unwrap();
}
