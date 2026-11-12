use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_contracts::{
    ApprovalRuleRegistry, BlockingService, Register, ToolPresenterRegistry, ToolRegistry,
};
use hanafubuki_agent_execution::AgentExecutionService;
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use kardia_core::{Context, Disposer, Inject, PluginDef, Service};

use crate::approval::GitCloneRequiresUser;
use crate::presenter::ExecPresenter;
use crate::tool::{ExecServices, ExecTool, ExecToolKind};

/// How long a stopping plugin waits for its output monitors and cleanup
/// retries; each observes cancellation within one poll.
const MONITOR_STOP_BOUND: Duration = Duration::from_secs(3);

/// Registers the execution tools over `executions`, which the host owns: it
/// also serves background terminals and is shut down after the plugins. Also
/// registers their presenter and the approval rule of the commands they
/// run.
pub fn execution_plugin(executions: AgentExecutionService) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let executions = executions.clone();
        async move {
            let tools = ctx.get::<ToolRegistry>()?;
            let presenters = ctx.get::<ToolPresenterRegistry>()?;
            let rules = ctx.get::<ApprovalRuleRegistry>()?;
            let blocking = ctx.get::<BlockingService>()?.arc().clone();
            // Agent threads: a panicking monitor ends only itself.
            let workers = BackgroundWorkerGroup::contained();
            let services = ExecServices {
                executions,
                workers: workers.clone(),
                blocking: blocking.clone(),
            };
            ctx.defer(
                "execution monitors",
                Disposer::future(move || async move {
                    let stopped = blocking
                        .run("agent execution monitors shutdown", move || {
                            workers.shutdown(Instant::now() + MONITOR_STOP_BOUND)
                        })
                        .await;
                    match stopped {
                        Ok(diagnostics) => {
                            for diagnostic in diagnostics {
                                log::warn!(
                                    "Agent execution worker {} did not stop: {}",
                                    diagnostic.worker,
                                    diagnostic.message
                                );
                            }
                        }
                        Err(error) => log::warn!("Agent execution workers did not stop: {error}"),
                    }
                }),
            )?;
            for kind in ExecToolKind::ALL {
                let tool =
                    ExecTool::new(kind, services.clone()).map_err(kardia_core::Error::new)?;
                tools.register(Arc::new(tool))?;
            }
            presenters.register(Arc::new(ExecPresenter))?;
            rules.register(Arc::new(GitCloneRequiresUser))?;
            Ok(())
        }
    })
    .name("execution")
    .inject(
        Inject::new()
            .with(ToolRegistry::NAME)
            .with(ToolPresenterRegistry::NAME)
            .with(ApprovalRuleRegistry::NAME)
            .with(BlockingService::NAME),
    )
}
