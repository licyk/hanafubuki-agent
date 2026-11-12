use std::sync::Arc;

use hanafubuki_agent_contracts::{BlockingService, Register, ToolPresenterRegistry, ToolRegistry};
use hanafubuki_agent_workspace_fs::WorkspaceFsService;
use kardia_core::{Context, Inject, PluginDef, Service};

use crate::presenter::FsPresenter;
use crate::tool::{FsTool, FsToolKind};

/// Registers the workspace filesystem tools over `files`, and their
/// presenter, whose policy (the
/// workspace roots and the protected application directories) the host
/// resolved.
pub fn workspace_fs_plugin(files: WorkspaceFsService) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let files = files.clone();
        async move {
            let tools = ctx.get::<ToolRegistry>()?;
            let presenters = ctx.get::<ToolPresenterRegistry>()?;
            let blocking = ctx.get::<BlockingService>()?.arc().clone();
            for kind in FsToolKind::ALL {
                let tool = FsTool::new(kind, files.clone(), blocking.clone())
                    .map_err(kardia_core::Error::new)?;
                tools.register(Arc::new(tool))?;
            }
            presenters.register(Arc::new(FsPresenter))?;
            Ok(())
        }
    })
    .name("workspace-fs")
    .inject(
        Inject::new()
            .with(ToolRegistry::NAME)
            .with(ToolPresenterRegistry::NAME)
            .with(BlockingService::NAME),
    )
}
