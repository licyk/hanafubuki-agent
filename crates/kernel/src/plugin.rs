use std::sync::Arc;

use hanafubuki_agent_contracts::{
    ApprovalRuleRegistry, ContextRegistry, ContextRendererRegistry, ContinuationHookRegistry,
    ObserverRegistry, PluginFaults, PromptContributorRegistry, RegistryService,
    ToolLifecycleHookRegistry, ToolRegistry, ToolSourceRegistry, TurnAdmissionRegistry,
};
use kardia_core::{Context, PluginDef, Service};

/// The registries the kernel reads.
///
/// They are created before the plugin runtime starts and live as long as the
/// kernel: the kernel plugin publishes them and never replaces them, so no
/// plugin restart can reach the kernel's own state.
#[derive(Clone)]
pub struct AgentKernelRegistries {
    pub tools: Arc<ToolRegistry>,
    /// The sources that plan tools per sampling step.
    pub tool_sources: Arc<ToolSourceRegistry>,
    /// The world-state contributors, collected at turn admission.
    pub context: Arc<ContextRegistry>,
    /// How each kind of section is shown, snapshotted per model request. It
    /// holds the kernel's own kinds' renderers from the start.
    pub context_renderers: Arc<ContextRendererRegistry>,
    /// The contributors of a turn's extensions, collected at turn admission.
    pub turn_admission: Arc<TurnAdmissionRegistry>,
    /// The prompt fragments' contributors, snapshotted per sampling step.
    pub prompt: Arc<PromptContributorRegistry>,
    /// Rules that tighten approval, asked for every action that needs it.
    pub approval_rules: Arc<ApprovalRuleRegistry>,
    /// The hooks asked before a final answer ends a turn, snapshotted per
    /// sampling step.
    pub continuation_hooks: Arc<ContinuationHookRegistry>,
    /// The hooks asked around each tool call, frozen with each sampling
    /// step's tool plan.
    pub tool_lifecycle_hooks: Arc<ToolLifecycleHookRegistry>,
    /// Told what the kernel committed, after the commit is durable.
    pub observers: Arc<ObserverRegistry>,
    /// Where every registry's entries, and plugins' scoped workers, report a
    /// panic in plugin code; the host stops the plugin it names.
    pub faults: Arc<PluginFaults>,
}

impl Default for AgentKernelRegistries {
    /// Registries sharing one [`PluginFaults`], empty but for the kernel's
    /// own renderers.
    fn default() -> Self {
        let faults = Arc::new(PluginFaults::default());
        let context_renderers = Arc::new(ContextRendererRegistry::with_faults(faults.clone()));
        for renderer in crate::model::kernel_context_renderers() {
            context_renderers
                .registry()
                .register_permanent("agent-kernel", renderer, ContextRendererRegistry::admit)
                .expect("the kernel's renderers are distinct kinds");
        }
        Self {
            tools: Arc::new(ToolRegistry::with_faults(faults.clone())),
            tool_sources: Arc::new(ToolSourceRegistry::with_faults(faults.clone())),
            context: Arc::new(ContextRegistry::with_faults(faults.clone())),
            context_renderers,
            turn_admission: Arc::new(TurnAdmissionRegistry::with_faults(faults.clone())),
            prompt: Arc::new(PromptContributorRegistry::with_faults(faults.clone())),
            approval_rules: Arc::new(ApprovalRuleRegistry::with_faults(faults.clone())),
            continuation_hooks: Arc::new(ContinuationHookRegistry::with_faults(faults.clone())),
            tool_lifecycle_hooks: Arc::new(ToolLifecycleHookRegistry::with_faults(faults.clone())),
            observers: Arc::new(ObserverRegistry::with_faults(faults.clone())),
            faults,
        }
    }
}

/// The kernel's kardia plugin. It takes no configuration and is mounted
/// first; every capability plugin injects the registries it provides.
pub fn kernel_plugin(registries: AgentKernelRegistries) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let registries = registries.clone();
        async move {
            ctx.provide_arc(registries.tools)?;
            ctx.provide_arc(registries.tool_sources)?;
            ctx.provide_arc(registries.context)?;
            ctx.provide_arc(registries.context_renderers)?;
            ctx.provide_arc(registries.turn_admission)?;
            ctx.provide_arc(registries.prompt)?;
            ctx.provide_arc(registries.approval_rules)?;
            ctx.provide_arc(registries.continuation_hooks)?;
            ctx.provide_arc(registries.tool_lifecycle_hooks)?;
            ctx.provide_arc(registries.observers)?;
            ctx.provide_arc(registries.faults)?;
            Ok(())
        }
    })
    .name("agent-kernel")
    .provides([
        ToolRegistry::NAME,
        ToolSourceRegistry::NAME,
        ContextRegistry::NAME,
        ContextRendererRegistry::NAME,
        TurnAdmissionRegistry::NAME,
        PromptContributorRegistry::NAME,
        ApprovalRuleRegistry::NAME,
        ContinuationHookRegistry::NAME,
        ToolLifecycleHookRegistry::NAME,
        ObserverRegistry::NAME,
        PluginFaults::NAME,
    ])
}
