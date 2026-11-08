//! The workspace-fs plugin registers its six tools while it runs and removes
//! them when it stops.

use std::sync::Arc;

use hanafubuki_agent_contracts::{BlockingService, RegistryService, ToolEffect, ToolRegistry};
use hanafubuki_agent_foundation::blocking::InlineBlockingOperationPort;
use hanafubuki_agent_plugin_workspace_fs::workspace_fs_plugin;
use hanafubuki_agent_tool_contracts::ToolExecutionClass;
use hanafubuki_agent_workspace_fs::{WorkspaceFsPolicy, WorkspaceFsService};
use kardia_core::{Context, FiberState, PluginDef, Service};
use serde_json::Value;

async fn root_with_host_services() -> (Context, Arc<ToolRegistry>) {
    let tools = Arc::new(ToolRegistry::default());
    let provided = tools.clone();
    let root = Context::new();
    let host = PluginDef::new(move |ctx: Context, _| {
        let tools = provided.clone();
        async move {
            ctx.provide_arc(tools)?;
            ctx.provide(BlockingService::new(Arc::new(InlineBlockingOperationPort)))?;
            Ok(())
        }
    })
    .provides([ToolRegistry::NAME, BlockingService::NAME]);
    root.plugin_def(&host, Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    (root, tools)
}

#[tokio::test]
async fn registers_the_filesystem_tools_and_removes_them_when_stopped() {
    let workspace = tempfile::tempdir().unwrap();
    let files = WorkspaceFsService::new(
        WorkspaceFsPolicy::rooted_at(workspace.path(), std::iter::empty::<std::path::PathBuf>())
            .unwrap(),
    );
    let (root, tools) = root_with_host_services().await;
    let fiber = root
        .plugin_def(&workspace_fs_plugin(files), Value::Null)
        .unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);

    let snapshot = tools.registry().snapshot();
    let mut registered = snapshot
        .items()
        .map(|tool| {
            let descriptor = tool.descriptor();
            (descriptor.name.qualified(), descriptor.effect)
        })
        .collect::<Vec<_>>();
    registered.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        registered,
        [
            ("apply_patch".to_string(), ToolEffect::Mutating),
            ("delete_path".to_string(), ToolEffect::Mutating),
            ("list_directory".to_string(), ToolEffect::ReadOnly),
            ("move_path".to_string(), ToolEffect::Mutating),
            ("read_file".to_string(), ToolEffect::ReadOnly),
            ("search_text".to_string(), ToolEffect::ReadOnly),
        ]
    );
    assert!(snapshot
        .items()
        .all(|tool| tool.descriptor().execution_class == ToolExecutionClass::Exclusive));

    fiber.dispose().await.unwrap();
    assert!(tools.registry().snapshot().is_empty());
    root.fiber().dispose().await.unwrap();
}
