use std::sync::Arc;

use hanafubuki_agent_contracts::{
    ArgumentSubjects, Register, ToolPresenterRegistry, ToolRegistry, ToolSourceRegistry,
};
use hanafubuki_agent_mcp::McpManager;
use kardia_core::{Context, Inject, PluginDef, Service};

use crate::resources::{McpResourceTool, McpResourceToolKind};
use crate::source::McpToolSource;

/// The `mcp` plugin: registers the MCP tool source, the resource tools and
/// their presenter over `mcp`. The application owns the manager and its lifecycle (servers,
/// OAuth, shutdown); the plugin only plans and runs tools through it.
pub fn mcp_plugin(mcp: Arc<McpManager>) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let mcp = mcp.clone();
        async move {
            let sources = ctx.get::<ToolSourceRegistry>()?;
            let tools = ctx.get::<ToolRegistry>()?;
            sources.register(Arc::new(McpToolSource { mcp }))?;
            for kind in McpResourceToolKind::ALL {
                let tool = McpResourceTool::new(kind).map_err(kardia_core::Error::new)?;
                tools.register(Arc::new(tool))?;
            }
            ctx.get::<ToolPresenterRegistry>()?
                .register(Arc::new(ArgumentSubjects::new(&[
                    ("list_mcp_resources", "server"),
                    ("list_mcp_resource_templates", "server"),
                    ("read_mcp_resource", "uri"),
                ])))?;
            Ok(())
        }
    })
    .name("mcp")
    .inject(
        Inject::new()
            .with(ToolSourceRegistry::NAME)
            .with(ToolRegistry::NAME)
            .with(ToolPresenterRegistry::NAME),
    )
}
