//! The hooks plugin registers its tool lifecycle hook, its Stop
//! continuation hook, its context contributor and renderer, and provides its
//! service while it runs; with no hook configured none of them applies; a
//! hook the user trusted through the service applies; and everything leaves
//! when the plugin stops.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentContinuationContext, AgentGeneration, AgentRevision, AgentSessionId, AgentTurnId,
    BlockingService, ContextRegistry, ContextRendererRegistry, ContinuationHookRegistry,
    RegistryService, ToolLifecycleCall, ToolLifecycleHookRegistry, ToolLifecyclePhase,
};
use hanafubuki_agent_foundation::blocking::InlineBlockingOperationPort;
use hanafubuki_agent_hooks::{HookSources, HookTrust, HooksEngine};
use hanafubuki_agent_plugin_hooks::{hooks_plugin, HooksService, WorkingDirectory, HOOKS_PLUGIN};
use hanafubuki_agent_process::executor::ExecutorManager;
use kardia_core::{Context, FiberState, PluginDef, Service};
use serde_json::{json, Value};

fn call(tool: &str) -> ToolLifecycleCall {
    ToolLifecycleCall {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(1),
        generation: AgentGeneration::new(1),
        call_id: "call-1".to_string(),
        tool_name: tool.to_string(),
        arguments: json!({}),
    }
}

#[tokio::test]
async fn registers_its_hooks_and_service_while_it_runs() {
    let directory = tempfile::tempdir().unwrap();
    let root = Context::new();
    let host = PluginDef::new(|ctx: Context, _| async move {
        ctx.provide(BlockingService::new(Arc::new(InlineBlockingOperationPort)))?;
        ctx.provide(ToolLifecycleHookRegistry::default())?;
        ctx.provide(ContinuationHookRegistry::default())?;
        ctx.provide(ContextRegistry::default())?;
        ctx.provide(ContextRendererRegistry::default())?;
        Ok(())
    })
    .provides([
        BlockingService::NAME,
        ToolLifecycleHookRegistry::NAME,
        ContinuationHookRegistry::NAME,
        ContextRegistry::NAME,
        ContextRendererRegistry::NAME,
    ]);
    root.plugin_def(&host, Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    let engine = Arc::new(HooksEngine::new(
        HookSources {
            user: directory.path().to_path_buf(),
            project: None,
        },
        directory.path().join("hooks-state.json"),
        ExecutorManager::default(),
    ));
    let cwd = directory.path().to_path_buf();
    let working_directory: WorkingDirectory = Arc::new(move || Ok(cwd.clone()));
    let fiber = root
        .plugin_def(&hooks_plugin(engine, working_directory), Value::Null)
        .unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);

    let tool_hooks = root
        .get::<ToolLifecycleHookRegistry>()
        .unwrap()
        .arc()
        .clone();
    let stop_hooks = root
        .get::<ContinuationHookRegistry>()
        .unwrap()
        .arc()
        .clone();
    let contributors = root.get::<ContextRegistry>().unwrap().arc().clone();
    let renderers = root.get::<ContextRendererRegistry>().unwrap().arc().clone();
    let [tool_hook] = &tool_hooks.ordered()[..] else {
        panic!("one tool lifecycle hook")
    };
    let [stop_hook] = &stop_hooks.ordered()[..] else {
        panic!("one continuation hook")
    };
    assert_eq!(tool_hook.item().provider_id.as_str(), HOOKS_PLUGIN);
    assert_eq!(contributors.registry().snapshot().entries().len(), 1);
    assert_eq!(renderers.registry().snapshot().entries().len(), 1);
    let stop_context = AgentContinuationContext {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(1),
        generation: AgentGeneration::new(1),
        revision: AgentRevision::new(1),
        sampling_step: 1,
        candidate_final_answer: "done".to_string(),
    };
    let applies = || {
        (
            tool_hook
                .item()
                .hook
                .applies(ToolLifecyclePhase::PreToolUse, &call("exec_command")),
            stop_hook.item().hook.applies(&stop_context),
        )
    };
    assert_eq!(applies(), (false, false), "no hook is configured");

    std::fs::write(
        directory.path().join("hooks.json"),
        serde_json::to_vec(&json!({ "hooks": {
            "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "true" }] }],
            "Stop": [{ "hooks": [{ "type": "command", "command": "true" }] }],
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = root
        .live_as::<HooksService>(HOOKS_PLUGIN)
        .unwrap()
        .arc()
        .clone();
    let listed = service.list().await.unwrap();
    assert_eq!(listed.hooks.len(), 2);
    assert_eq!(
        applies(),
        (false, false),
        "an untrusted hook does not apply"
    );
    service
        .trust(listed.hooks.iter().map(|hook| hook.key.clone()).collect())
        .await
        .unwrap();
    assert!(service
        .list()
        .await
        .unwrap()
        .hooks
        .iter()
        .all(|hook| hook.trust == HookTrust::Trusted));
    assert_eq!(applies(), (true, true));
    assert!(!tool_hook
        .item()
        .hook
        .applies(ToolLifecyclePhase::PreToolUse, &call("read_file")));

    fiber.dispose().await.unwrap();
    assert!(tool_hooks.registry().snapshot().is_empty());
    assert!(stop_hooks.registry().snapshot().is_empty());
    assert!(contributors.registry().snapshot().is_empty());
    assert!(renderers.registry().snapshot().is_empty());
    assert!(root.live_as::<HooksService>(HOOKS_PLUGIN).is_none());
    root.fiber().dispose().await.unwrap();
}
