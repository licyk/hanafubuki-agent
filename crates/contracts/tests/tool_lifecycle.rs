//! Tool lifecycle hooks are asked in `(priority, provider id, hook id)`
//! order whatever order their plugins registered them in; the registry
//! admits one hook per identity and at most `MAXIMUM_TOOL_LIFECYCLE_HOOKS`,
//! and a stopping plugin's hooks leave it.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentContinuationHookGeneration, AgentContinuationHookId, AgentContinuationHookProviderId,
    PostToolUseDecision, PostToolUseFuture, PreToolUseDecision, PreToolUseFuture, Register,
    RegistrationError, RegistryService, ToolLifecycleCall, ToolLifecycleHook,
    ToolLifecycleHookRegistration, ToolLifecycleHookRegistry, ToolLifecyclePhase,
    MAXIMUM_TOOL_LIFECYCLE_HOOKS,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_tool_contracts::ToolOutput;
use kardia_core::{Context, FiberState, Inject, PluginDef, Service};
use serde_json::Value;

struct AllowHook;

impl ToolLifecycleHook for AllowHook {
    fn applies(&self, _phase: ToolLifecyclePhase, _call: &ToolLifecycleCall) -> bool {
        true
    }

    fn before_tool_use<'a>(
        &'a self,
        _call: ToolLifecycleCall,
        _cancellation: CancellationToken,
    ) -> PreToolUseFuture<'a> {
        Box::pin(async { Ok(PreToolUseDecision::Allow) })
    }

    fn after_tool_use<'a>(
        &'a self,
        _call: ToolLifecycleCall,
        _output: ToolOutput,
        _cancellation: CancellationToken,
    ) -> PostToolUseFuture<'a> {
        Box::pin(async { Ok(PostToolUseDecision::Pass) })
    }
}

fn registration(provider: &str, hook: &str, priority: i32) -> ToolLifecycleHookRegistration {
    ToolLifecycleHookRegistration {
        provider_id: AgentContinuationHookProviderId::parse(provider.to_string()).unwrap(),
        hook_id: AgentContinuationHookId::parse(hook.to_string()).unwrap(),
        generation: AgentContinuationHookGeneration([0; 32]),
        priority,
        hook: Arc::new(AllowHook),
    }
}

/// The hook ids of a registry filled by one plugin per registration, in
/// `registrations` order, as the kernel would ask them.
async fn asked_order(registrations: &[(&'static str, &'static str, i32)]) -> Vec<String> {
    let root = Context::new();
    root.plugin_def(
        &PluginDef::new(|ctx: Context, _| async move {
            ctx.provide(ToolLifecycleHookRegistry::default())?;
            Ok(())
        })
        .provides([ToolLifecycleHookRegistry::NAME]),
        Value::Null,
    )
    .unwrap()
    .settled()
    .await
    .unwrap();
    for (index, (provider, hook, priority)) in registrations.iter().copied().enumerate() {
        let plugin = PluginDef::new(move |ctx: Context, _| async move {
            let hooks = ctx.get::<ToolLifecycleHookRegistry>()?;
            hooks.register(registration(provider, hook, priority))?;
            Ok(())
        })
        .name(format!("hooks-{index}"))
        .inject(Inject::new().with(ToolLifecycleHookRegistry::NAME));
        let fiber = root.plugin_def(&plugin, Value::Null).unwrap();
        fiber.settled().await.unwrap();
        assert_eq!(fiber.state(), FiberState::Active);
    }
    let hooks = root
        .get::<ToolLifecycleHookRegistry>()
        .unwrap()
        .arc()
        .clone();
    let order = hooks
        .ordered()
        .iter()
        .map(|entry| {
            let item = entry.item();
            format!("{}/{}", item.provider_id.as_str(), item.hook_id.as_str())
        })
        .collect();
    root.fiber().dispose().await.unwrap();
    assert!(hooks.registry().snapshot().is_empty());
    order
}

#[tokio::test]
async fn hooks_are_asked_by_priority_then_identity_not_registration_order() {
    let registrations = [
        ("z", "late", 20),
        ("b", "early", 10),
        ("a", "second", 10),
        ("a", "first", 10),
    ];
    let expected = ["a/first", "a/second", "b/early", "z/late"];
    assert_eq!(asked_order(&registrations).await, expected);
    let mut reversed = registrations.to_vec();
    reversed.reverse();
    assert_eq!(asked_order(&reversed).await, expected);
}

#[test]
fn the_registry_admits_one_hook_per_identity_up_to_the_limit() {
    let existing = registration("provider", "hook", 1);
    assert!(matches!(
        ToolLifecycleHookRegistry::admit(&[&existing], &registration("provider", "hook", 2)),
        Err(RegistrationError::Duplicate { identity, .. }) if identity == "provider/hook"
    ));
    assert!(
        ToolLifecycleHookRegistry::admit(&[&existing], &registration("provider", "other", 2))
            .is_ok()
    );

    let full = (0..MAXIMUM_TOOL_LIFECYCLE_HOOKS)
        .map(|index| registration("provider", &format!("hook-{index}"), 0))
        .collect::<Vec<_>>();
    let full = full.iter().collect::<Vec<_>>();
    assert!(matches!(
        ToolLifecycleHookRegistry::admit(&full, &registration("other", "hook", 0)),
        Err(RegistrationError::Rejected { .. })
    ));
}
