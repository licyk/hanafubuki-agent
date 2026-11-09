//! The execution plugin registers its tools and its approval rule while it
//! runs and removes them, and stops its output monitors, when it stops.

use std::sync::Arc;

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, PermissionActionDetails,
};
use hanafubuki_agent_contracts::{
    ApprovalRuleRegistry, ApprovalRuleVerdict, BlockingService, RegistryService, ToolEffect,
    ToolRegistry,
};
use hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort;
use hanafubuki_agent_execution::AgentExecutionService;
use hanafubuki_agent_foundation::blocking::InlineBlockingOperationPort;
use hanafubuki_agent_plugin_execution::execution_plugin;
use kardia_core::{Context, Fiber, FiberState, PluginDef, Service};
use serde_json::Value;

struct Mounted {
    root: Context,
    fiber: Fiber,
    tools: Arc<ToolRegistry>,
    rules: Arc<ApprovalRuleRegistry>,
}

async fn mount() -> Mounted {
    let tools = Arc::new(ToolRegistry::default());
    let rules = Arc::new(ApprovalRuleRegistry::default());
    let provided = (tools.clone(), rules.clone());
    let root = Context::new();
    let host = PluginDef::new(move |ctx: Context, _| {
        let (tools, rules) = provided.clone();
        async move {
            ctx.provide_arc(tools)?;
            ctx.provide_arc(rules)?;
            ctx.provide(BlockingService::new(Arc::new(InlineBlockingOperationPort)))?;
            Ok(())
        }
    })
    .provides([
        ToolRegistry::NAME,
        ApprovalRuleRegistry::NAME,
        BlockingService::NAME,
    ]);
    root.plugin_def(&host, Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();

    let executions =
        AgentExecutionService::with_process_port(FakeAgentExecutionProcessPort::shared());
    let fiber = root
        .plugin_def(&execution_plugin(executions), Value::Null)
        .unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);
    Mounted {
        root,
        fiber,
        tools,
        rules,
    }
}

fn command_action(kind: ApprovalActionKind, program: &str, argv: &[&str]) -> ApprovalAction {
    ApprovalAction::from_canonical_bytes(
        kind,
        ApprovalRiskClass::Critical,
        ApprovalOrigin::Model,
        format!("run {program}"),
        argv.join(" ").as_bytes(),
    )
    .and_then(|action| {
        action.with_details(PermissionActionDetails::Command {
            program: program.to_string(),
            argv: argv.iter().map(|argument| argument.to_string()).collect(),
            cwd: "/workspace".to_string(),
        })
    })
    .unwrap()
}

#[tokio::test]
async fn registers_the_execution_tools_and_removes_them_when_stopped() {
    let mounted = mount().await;
    let mut names = mounted
        .tools
        .registry()
        .snapshot()
        .items()
        .map(|tool| {
            assert_eq!(tool.descriptor().effect, ToolEffect::Mutating);
            tool.descriptor().name.qualified()
        })
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["exec_command", "resize_execution", "write_stdin"]);

    mounted.fiber.dispose().await.unwrap();
    assert!(mounted.tools.registry().snapshot().is_empty());
    assert!(mounted.rules.registry().snapshot().is_empty());
    mounted.root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn a_git_clone_always_requires_a_person() {
    let mounted = mount().await;
    for program in ["git", "/usr/bin/git", "C:\\Git\\cmd\\git.exe"] {
        for kind in [
            ApprovalActionKind::CommandExecution,
            ApprovalActionKind::ShellExecution,
        ] {
            let action = command_action(kind, program, &[program, "clone", "https://example"]);
            assert_eq!(
                mounted.rules.snapshot().verdict(None, &action),
                Some(ApprovalRuleVerdict::RequireUser),
                "{program}"
            );
        }
    }
    // The rule reads the program and its arguments, never a shell script.
    for (program, argv) in [
        ("git", ["git", "pull"].as_slice()),
        ("/usr/bin/legit", ["/usr/bin/legit", "clone"].as_slice()),
        (
            "bash",
            ["bash", "-c", "git clone https://example"].as_slice(),
        ),
    ] {
        let action = command_action(ApprovalActionKind::CommandExecution, program, argv);
        assert_eq!(
            mounted.rules.snapshot().verdict(None, &action),
            None,
            "{argv:?}"
        );
    }
    let unrelated = ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::FileWrite,
        ApprovalRiskClass::High,
        ApprovalOrigin::Model,
        "write a file".to_string(),
        b"file",
    )
    .unwrap();
    assert_eq!(mounted.rules.snapshot().verdict(None, &unrelated), None);
    mounted.root.fiber().dispose().await.unwrap();
}
