//! Plugins mounted on their own kardia root with the host services and
//! registries the Agent host provides, and the tool call context the kernel
//! would hand a call.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    ApprovalRuleRegistry, BlockingService, ContextRegistry, ContextRendererRegistry,
    ObserverRegistry, PromptContributorRegistry, RegistryService, ToolCallContext, ToolInvocation,
    ToolPresenterRegistry, ToolRegistry, ToolResult, ToolSourceRegistry, TurnAdmissionRegistry,
    TurnExtensions,
};
use hanafubuki_agent_foundation::blocking::{BlockingOperationPort, InlineBlockingOperationPort};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{AgentToolError, AgentToolInvocationContext, AuthorizedAction};
use kardia_core::{Context, Fiber, FiberState, PluginDef, Service};
use serde_json::{json, Value};

pub fn test_tool_context() -> AgentToolInvocationContext {
    AgentToolInvocationContext {
        session_id: serde_json::from_value(json!(1)).unwrap(),
        turn_id: serde_json::from_value(json!(1)).unwrap(),
        generation: serde_json::from_value(json!(1)).unwrap(),
        mode: hanafubuki_agent_kernel::AgentSessionMode::Default,
        input_capabilities: hanafubuki_agent_kernel::ModelInputCapabilities::default(),
        session_revision: serde_json::from_value(json!(1)).unwrap(),
        call_id: "test-call".to_string(),
        catalog_revision: hanafubuki_agent_kernel::AgentToolCatalogRevision([1; 32]),
        provider_id: hanafubuki_agent_kernel::AgentToolProviderId::parse("builtin").unwrap(),
        tool_id: hanafubuki_agent_kernel::AgentToolId::parse("test-tool".to_string()).unwrap(),
        tool_generation: hanafubuki_agent_kernel::AgentToolGeneration([2; 32]),
        origin: Default::default(),
        tool_plan_leases: Vec::new(),
        approval_rules: None,
        progress: Arc::new(hanafubuki_agent_kernel::NoopAgentToolProgressSink),
    }
}

/// A tool plugin running on its own kardia root with the host services the
/// Agent host provides.
pub struct MountedToolPlugin {
    tools: Arc<ToolRegistry>,
    _root: Context,
}

pub async fn mount_tool_plugin(
    plugin: PluginDef,
    blocking: Arc<dyn BlockingOperationPort>,
) -> MountedToolPlugin {
    let tools = Arc::new(ToolRegistry::default());
    let provided = tools.clone();
    let root = Context::new();
    let host = PluginDef::new(move |ctx: Context, _| {
        let tools = provided.clone();
        let blocking = blocking.clone();
        async move {
            ctx.provide_arc(tools)?;
            ctx.provide(ApprovalRuleRegistry::default())?;
            ctx.provide(TurnAdmissionRegistry::default())?;
            ctx.provide(PromptContributorRegistry::default())?;
            ctx.provide(ContextRendererRegistry::default())?;
            ctx.provide(ToolPresenterRegistry::default())?;
            ctx.provide(BlockingService::new(blocking))?;
            Ok(())
        }
    })
    .provides([
        ToolRegistry::NAME,
        ToolPresenterRegistry::NAME,
        ApprovalRuleRegistry::NAME,
        TurnAdmissionRegistry::NAME,
        PromptContributorRegistry::NAME,
        ContextRendererRegistry::NAME,
        BlockingService::NAME,
    ]);
    root.plugin_def(&host, Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    let fiber = root.plugin_def(&plugin, Value::Null).unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);
    MountedToolPlugin { tools, _root: root }
}

impl MountedToolPlugin {
    /// Runs the registered tool `name` the way the kernel does once its
    /// permission is settled, with `approval` as the consumed approval.
    pub async fn invoke(
        &self,
        name: &str,
        arguments: Value,
        approval: Option<AuthorizedAction>,
    ) -> Result<ToolResult, AgentToolError> {
        self.invoke_in_turn(name, arguments, approval, TurnExtensions::default())
            .await
    }

    /// [`Self::invoke`] with the turn data the kernel would hand the call.
    pub async fn invoke_in_turn(
        &self,
        name: &str,
        arguments: Value,
        approval: Option<AuthorizedAction>,
        turn: TurnExtensions,
    ) -> Result<ToolResult, AgentToolError> {
        let tool = self
            .tools
            .registry()
            .snapshot()
            .items()
            .find(|tool| tool.descriptor().name.qualified() == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} is registered"));
        let context = test_tool_context();
        tool.invoke(
            ToolInvocation {
                context: ToolCallContext {
                    session_id: context.session_id,
                    turn_id: context.turn_id,
                    generation: context.generation,
                    mode: context.mode,
                    session_revision: context.session_revision,
                    call_id: context.call_id,
                    image_input: false,
                    origin: Default::default(),
                },
                arguments,
                cancellation: CancellationToken::default(),
                progress: context.progress,
                plan_leases: context.tool_plan_leases,
                turn,
            },
            approval,
        )
        .await
    }
}

/// `plugin` running on a fresh root that provides the host services the
/// Agent host provides.
pub struct MountedPlugin {
    pub root: Context,
    pub fiber: Fiber,
    pub tools: Arc<ToolRegistry>,
    pub sources: Arc<ToolSourceRegistry>,
    pub context: Arc<ContextRegistry>,
    pub renderers: Arc<ContextRendererRegistry>,
    pub admission: Arc<TurnAdmissionRegistry>,
    pub prompt: Arc<PromptContributorRegistry>,
    pub rules: Arc<ApprovalRuleRegistry>,
    pub observers: Arc<ObserverRegistry>,
    pub presenters: Arc<ToolPresenterRegistry>,
}

pub async fn mount(plugin: PluginDef) -> MountedPlugin {
    let tools = Arc::new(ToolRegistry::default());
    let sources = Arc::new(ToolSourceRegistry::default());
    let context = Arc::new(ContextRegistry::default());
    let renderers = Arc::new(ContextRendererRegistry::default());
    let admission = Arc::new(TurnAdmissionRegistry::default());
    let prompt = Arc::new(PromptContributorRegistry::default());
    let rules = Arc::new(ApprovalRuleRegistry::default());
    let observers = Arc::new(ObserverRegistry::default());
    let presenters = Arc::new(ToolPresenterRegistry::default());
    let provided = (
        tools.clone(),
        sources.clone(),
        (context.clone(), renderers.clone()),
        (admission.clone(), prompt.clone()),
        rules.clone(),
        (observers.clone(), presenters.clone()),
    );
    let root = Context::new();
    let host = PluginDef::new(move |ctx: Context, _| {
        let (
            tools,
            sources,
            (context, renderers),
            (admission, prompt),
            rules,
            (observers, presenters),
        ) = provided.clone();
        async move {
            ctx.provide_arc(tools)?;
            ctx.provide_arc(sources)?;
            ctx.provide_arc(context)?;
            ctx.provide_arc(renderers)?;
            ctx.provide_arc(admission)?;
            ctx.provide_arc(prompt)?;
            ctx.provide_arc(rules)?;
            ctx.provide_arc(observers)?;
            ctx.provide_arc(presenters)?;
            ctx.provide(BlockingService::new(Arc::new(InlineBlockingOperationPort)))?;
            ctx.provide(hanafubuki_agent_contracts::PluginFaults::default())?;
            Ok(())
        }
    })
    .provides([
        hanafubuki_agent_contracts::PluginFaults::NAME,
        ToolRegistry::NAME,
        ToolSourceRegistry::NAME,
        ContextRegistry::NAME,
        ContextRendererRegistry::NAME,
        TurnAdmissionRegistry::NAME,
        PromptContributorRegistry::NAME,
        ApprovalRuleRegistry::NAME,
        ObserverRegistry::NAME,
        ToolPresenterRegistry::NAME,
        BlockingService::NAME,
    ]);
    root.plugin_def(&host, Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    let fiber = root.plugin_def(&plugin, Value::Null).unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);
    MountedPlugin {
        root,
        fiber,
        tools,
        sources,
        context,
        renderers,
        admission,
        prompt,
        rules,
        observers,
        presenters,
    }
}
