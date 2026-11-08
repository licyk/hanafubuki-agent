//! The mcp plugin registers its tool source and resource tools while it
//! runs and removes them when it stops.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, RegistryService, ToolEffect, ToolRegistry,
    ToolSourceRegistry,
};
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_mcp::McpManager;
use hanafubuki_agent_plugin_mcp::mcp_plugin;
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_tool_contracts::{ToolExecutionClass, ToolExposure};
use kardia_core::{Context, FiberState, PluginDef, Service};
use serde_json::Value;

async fn root_with_registries() -> (Context, Arc<ToolRegistry>, Arc<ToolSourceRegistry>) {
    let tools = Arc::new(ToolRegistry::default());
    let sources = Arc::new(ToolSourceRegistry::default());
    let provided = (tools.clone(), sources.clone());
    let root = Context::new();
    let registries = PluginDef::new(move |ctx: Context, _| {
        let (tools, sources) = provided.clone();
        async move {
            ctx.provide_arc(tools)?;
            ctx.provide_arc(sources)?;
            Ok(())
        }
    })
    .name("registries")
    .provides([ToolRegistry::NAME, ToolSourceRegistry::NAME]);
    root.plugin_def(&registries, Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    (root, tools, sources)
}

fn tool_names(tools: &ToolRegistry) -> Vec<String> {
    let mut names = tools
        .registry()
        .snapshot()
        .items()
        .map(|tool| tool.descriptor().name.qualified())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn source_ids(sources: &ToolSourceRegistry) -> Vec<String> {
    sources
        .registry()
        .snapshot()
        .items()
        .map(|source| source.id().to_string())
        .collect()
}

#[tokio::test]
async fn the_plugin_registers_the_source_and_the_resource_tools_until_it_stops() {
    let (root, tools, sources) = root_with_registries().await;
    let manager = Arc::new(McpManager::new(
        Arc::new(ExecutorManager::default()),
        BackgroundWorkerGroup::default(),
    ));
    let fiber = root.plugin_def(&mcp_plugin(manager), Value::Null).unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);
    assert_eq!(source_ids(&sources), ["mcp"]);
    assert_eq!(
        tool_names(&tools),
        [
            "list_mcp_resource_templates",
            "list_mcp_resources",
            "read_mcp_resource"
        ]
    );
    // Concurrent reads, offered in Plan mode, presented as generic tool use.
    for tool in tools.registry().snapshot().items() {
        let descriptor = tool.descriptor();
        assert_eq!(descriptor.exposure, ToolExposure::Direct);
        assert_eq!(
            descriptor.execution_class,
            ToolExecutionClass::ConcurrentRead
        );
        assert_eq!(descriptor.effect, ToolEffect::ReadOnly);
        assert_eq!(descriptor.action, AgentActivityAction::USE_TOOL);
        assert_eq!(descriptor.memory_effect, AgentMemoryContextEffect::None);
    }

    fiber.dispose().await.unwrap();
    assert!(sources.registry().snapshot().is_empty());
    assert!(tools.registry().snapshot().is_empty());
    root.fiber().dispose().await.unwrap();
}
