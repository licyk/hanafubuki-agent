use std::path::PathBuf;
use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentContinuationHookFailurePolicy, AgentContinuationHookGeneration, AgentContinuationHookId,
    AgentContinuationHookProviderId, AgentContinuationHookRegistration, BlockingService,
    ContextRegistry, ContextRendererRegistry, ContinuationHookRegistry, Register,
    ToolLifecycleHookRegistration, ToolLifecycleHookRegistry,
};
use hanafubuki_agent_hooks::HooksEngine;
use kardia_core::{Context, Inject, PluginDef, Service};
use sha2::{Digest, Sha256};

use crate::context::{hook_context_renderer, HookContext};
use crate::runtime::HooksRuntime;
use crate::service::HooksService;
use crate::stop::StopHooks;
use crate::tool_use::ToolUseHooks;

/// The plugin's name, under which it provides its [`HooksService`]; also
/// the provider id of its hooks.
pub const HOOKS_PLUGIN: &str = "hooks";

/// The directory the Agent works in, where a hook's command runs; called
/// on the blocking pool.
pub type WorkingDirectory = Arc<dyn Fn() -> Result<PathBuf, String> + Send + Sync>;

/// Binds `engine`'s hooks to the kernel and provides the [`HooksService`].
/// The application builds the engine over its own `ExecutorManager`.
pub fn hooks_plugin(engine: Arc<HooksEngine>, working_directory: WorkingDirectory) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let (engine, working_directory) = (engine.clone(), working_directory.clone());
        async move {
            let blocking = ctx.get::<BlockingService>()?.arc().clone();
            let runtime = Arc::new(HooksRuntime::new(
                engine,
                working_directory,
                blocking.clone(),
            ));
            let (provider_id, generation) = identity()?;
            ctx.get::<ToolLifecycleHookRegistry>()?
                .register(ToolLifecycleHookRegistration {
                    provider_id: provider_id.clone(),
                    hook_id: hook_id("tool-use")?,
                    generation,
                    priority: 0,
                    hook: Arc::new(ToolUseHooks(runtime.clone())),
                })?;
            // A broken Stop hook must not fail the turn it ran for.
            ctx.get::<ContinuationHookRegistry>()?
                .register(AgentContinuationHookRegistration {
                    provider_id,
                    hook_id: hook_id("stop")?,
                    generation,
                    priority: 0,
                    failure_policy: AgentContinuationHookFailurePolicy::Ignore,
                    hook: Arc::new(StopHooks(runtime.clone())),
                })?;
            ctx.get::<ContextRegistry>()?
                .register(Arc::new(HookContext(runtime.clone())))?;
            ctx.get::<ContextRendererRegistry>()?
                .register(hook_context_renderer())?;
            ctx.provide_arc_as(HOOKS_PLUGIN, Arc::new(HooksService { runtime, blocking }))?;
            Ok(())
        }
    })
    .name(HOOKS_PLUGIN)
    .provides([HOOKS_PLUGIN])
    .inject(
        Inject::new()
            .with(BlockingService::NAME)
            .with(ToolLifecycleHookRegistry::NAME)
            .with(ContinuationHookRegistry::NAME)
            .with(ContextRegistry::NAME)
            .with(ContextRendererRegistry::NAME),
    )
}

/// The provider id and the generation every fragment the plugin adds is
/// recorded with.
fn identity() -> Result<
    (
        AgentContinuationHookProviderId,
        AgentContinuationHookGeneration,
    ),
    kardia_core::Error,
> {
    let provider_id = AgentContinuationHookProviderId::parse(HOOKS_PLUGIN.to_string())
        .map_err(kardia_core::Error::msg)?;
    let generation =
        AgentContinuationHookGeneration(Sha256::digest(b"hanafubuki-agent-plugin-hooks/1").into());
    Ok((provider_id, generation))
}

fn hook_id(id: &str) -> Result<AgentContinuationHookId, kardia_core::Error> {
    AgentContinuationHookId::parse(id.to_string()).map_err(kardia_core::Error::msg)
}
