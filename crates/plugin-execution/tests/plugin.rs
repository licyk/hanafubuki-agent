//! The execution plugin registers its tools, their presenter, its
//! approval rule and its guidance while it runs and removes them, and stops its output monitors, when it stops.
//! Each command takes the command timeout its options set at its call; a
//! change of them is taken in place, and the guidance states it from the
//! next step.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hanafubuki_agent_approval::test_support::ManualApprovalClock;
use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalContext, ApprovalDecisionSource, ApprovalOrigin,
    ApprovalRiskClass, ApprovalService, AuthorizedAction, PermissionActionDetails,
    PermissionRequirement,
};
use hanafubuki_agent_contracts::{
    AgentExecutionProgressStatus, AgentGeneration, AgentRevision, AgentSessionId, AgentSessionMode,
    AgentToolHandler, AgentToolProgressSink, AgentToolProgressUpdate, AgentTurnId,
    ApprovalRuleRegistry, ApprovalRuleVerdict, BlockingService, ContextRendererRegistry,
    ContextSlot, PromptContributorRegistry, PromptFragment, PromptRequest, PromptStep,
    RegistryService, ToolCallContext, ToolCallOrigin, ToolEffect, ToolInvocation,
    ToolPresenterRegistry, ToolRegistry, TurnExtensions, TurnWake,
};
use hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort;
use hanafubuki_agent_execution::AgentExecutionService;
use hanafubuki_agent_foundation::blocking::InlineBlockingOperationPort;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_plugin_execution::execution_plugin;
use hanafubuki_agent_sandbox::SandboxSettings;
use kardia_core::{Context, Fiber, FiberState, PluginDef, Service};
use serde_json::{json, Value};

struct Mounted {
    root: Context,
    fiber: Fiber,
    tools: Arc<ToolRegistry>,
    presenters: Arc<ToolPresenterRegistry>,
    rules: Arc<ApprovalRuleRegistry>,
    prompts: Arc<PromptContributorRegistry>,
    renderers: Arc<ContextRendererRegistry>,
    process: Arc<FakeAgentExecutionProcessPort>,
}

async fn mount() -> Mounted {
    mount_with(Value::Null).await
}

/// The plugin over a fake process port, with `options` as its row's.
async fn mount_with(options: Value) -> Mounted {
    let tools = Arc::new(ToolRegistry::default());
    let presenters = Arc::new(ToolPresenterRegistry::default());
    let rules = Arc::new(ApprovalRuleRegistry::default());
    let prompts = Arc::new(PromptContributorRegistry::default());
    let renderers = Arc::new(ContextRendererRegistry::default());
    let provided = (
        tools.clone(),
        presenters.clone(),
        rules.clone(),
        prompts.clone(),
        renderers.clone(),
    );
    let root = Context::new();
    let host = PluginDef::new(move |ctx: Context, _| {
        let (tools, presenters, rules, prompts, renderers) = provided.clone();
        async move {
            ctx.provide_arc(tools)?;
            ctx.provide_arc(presenters)?;
            ctx.provide_arc(rules)?;
            ctx.provide_arc(prompts)?;
            ctx.provide_arc(renderers)?;
            ctx.provide(BlockingService::new(Arc::new(InlineBlockingOperationPort)))?;
            Ok(())
        }
    })
    .provides([
        ToolRegistry::NAME,
        ToolPresenterRegistry::NAME,
        ApprovalRuleRegistry::NAME,
        PromptContributorRegistry::NAME,
        ContextRendererRegistry::NAME,
        BlockingService::NAME,
    ]);
    root.plugin_def(&host, Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();

    let process = FakeAgentExecutionProcessPort::shared();
    let executions = AgentExecutionService::with_process_port(process.clone());
    let fiber = root
        .plugin_def(&execution_plugin(executions), options)
        .unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);
    Mounted {
        root,
        fiber,
        tools,
        presenters,
        rules,
        prompts,
        renderers,
        process,
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
    let command = serde_json::json!({ "cmd": "cargo test" });
    assert_eq!(
        mounted
            .presenters
            .subject("exec_command", &command)
            .as_deref(),
        Some("cargo test")
    );

    mounted.fiber.dispose().await.unwrap();
    assert!(mounted.tools.registry().snapshot().is_empty());
    assert!(mounted.presenters.registry().snapshot().is_empty());
    assert!(mounted.rules.registry().snapshot().is_empty());
    assert!(mounted.prompts.registry().snapshot().is_empty());
    assert!(mounted.renderers.registry().snapshot().is_empty());
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

#[derive(Default)]
struct RecordingProgress(Mutex<Vec<AgentToolProgressUpdate>>);

impl AgentToolProgressSink for RecordingProgress {
    fn report(&self, update: AgentToolProgressUpdate) {
        self.0.lock().unwrap().push(update);
    }
}

impl Mounted {
    fn exec_command(&self) -> Arc<dyn AgentToolHandler> {
        self.tools
            .registry()
            .snapshot()
            .items()
            .find(|tool| tool.descriptor().name.qualified() == "exec_command")
            .cloned()
            .unwrap()
    }

    /// The approval `invocation` asks for, which these tests give.
    async fn approval(&self, invocation: &ToolInvocation) -> (ApprovalAction, AuthorizedAction) {
        let PermissionRequirement::ReviewRequired(action) =
            self.exec_command().permission(invocation).await.unwrap()
        else {
            panic!("a command asks for its approval");
        };
        let context =
            ApprovalContext::from_canonical_bytes("call-1".to_string(), b"scope-1").unwrap();
        let approval = ApprovalService::new(Arc::new(ManualApprovalClock::new(1)))
            .authorize(
                action.clone(),
                &context,
                Duration::from_secs(1),
                ApprovalDecisionSource::FullAccess,
            )
            .unwrap();
        (action, approval)
    }

    /// Runs `invocation` with its approval and returns that approval's action.
    async fn run(&self, invocation: ToolInvocation) -> ApprovalAction {
        let (action, approval) = self.approval(&invocation).await;
        self.exec_command()
            .invoke(invocation, Some(approval))
            .await
            .unwrap();
        action
    }
}

/// `cmd` as the model (or, with `ToolCallOrigin::User`, the person) calls
/// it, yielding at once, in a turn with a sandbox.
fn command(cmd: &str, origin: ToolCallOrigin, progress: Arc<RecordingProgress>) -> ToolInvocation {
    ToolInvocation {
        context: ToolCallContext {
            session_id: AgentSessionId::new(1),
            turn_id: AgentTurnId::new(1),
            generation: AgentGeneration::new(1),
            mode: AgentSessionMode::Default,
            session_revision: AgentRevision::new(1),
            call_id: "call-1".to_string(),
            image_input: false,
            origin,
        },
        arguments: json!({
            "cmd": cmd,
            "yield_time_ms": 1,
            "workdir": std::env::current_dir().unwrap(),
        }),
        cancellation: CancellationToken::default(),
        progress,
        plan_leases: Vec::new(),
        turn: TurnExtensions::default().with(Arc::new(SandboxSettings::full_access())),
    }
}

const MINUTE_MS: u64 = 60_000;

/// A change of the timeout is taken in place: the plugin keeps running, so
/// the output monitor of the command already started goes on, and that
/// command keeps the timeout it started with; the next command takes the
/// new one, and so does its approval.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_takes_the_timeout_at_its_call_and_a_change_restarts_nothing() {
    let mounted = mount().await;
    let progress = Arc::new(RecordingProgress::default());
    let invocation = || command("serve", ToolCallOrigin::Model, progress.clone());
    let first = mounted.run(invocation()).await;
    assert_eq!(mounted.process.starts()[0].timeout_ms, Some(10 * MINUTE_MS));
    let tool = mounted.exec_command();

    mounted
        .fiber
        .update(json!({ "commandTimeoutMinutes": 90 }))
        .unwrap()
        .await
        .unwrap();
    assert_eq!(mounted.fiber.state(), FiberState::Active);
    assert!(
        Arc::ptr_eq(&tool, &mounted.exec_command()),
        "the plugin did not restart"
    );
    let first_process = mounted.process.starts()[0].executor_id.clone().unwrap();
    mounted
        .process
        .set_output(&first_process, b"after the change\n".to_vec());
    let deadline = Instant::now() + Duration::from_secs(30);
    while !progress.0.lock().unwrap().iter().any(|update| {
        matches!(update, AgentToolProgressUpdate::ExecutionOutput { text, .. }
            if text.contains("after the change"))
    }) {
        assert!(Instant::now() < deadline, "the monitor stopped");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let next = mounted.run(invocation()).await;
    let starts = mounted.process.starts();
    assert_eq!(starts.len(), 2, "nothing started again");
    assert_eq!(starts[0].timeout_ms, Some(10 * MINUTE_MS));
    assert_eq!(starts[1].timeout_ms, Some(90 * MINUTE_MS));
    assert_eq!(mounted.process.session_count(), 2, "the first still runs");
    assert_ne!(
        first.digest, next.digest,
        "the approval is of the timeout too"
    );
    mounted.root.fiber().dispose().await.unwrap();
}

/// A command the person typed (`!command`) takes the option too. It holds
/// its call until it exits, so the kernel's tool timeout and the turn's
/// deadline, outside this plugin, still stop it first when they are shorter.
#[tokio::test(flavor = "multi_thread")]
async fn a_typed_command_takes_the_options_timeout() {
    let mounted = mount_with(json!({ "commandTimeoutMinutes": 2 })).await;
    mounted.process.finish_on_start("typed\n", 0);
    let progress = Arc::new(RecordingProgress::default());
    let action = mounted
        .run(command("ls", ToolCallOrigin::User, progress))
        .await;
    assert_eq!(action.origin, ApprovalOrigin::User);
    assert_eq!(mounted.process.starts()[0].timeout_ms, Some(2 * MINUTE_MS));
    mounted.root.fiber().dispose().await.unwrap();
}

impl Mounted {
    /// What the plugin's guidance contributes to a step that offers
    /// `tools`, of a turn whose session is woken for its background work as
    /// `woken` says (no `TurnWake` when `None`).
    async fn guidance(&self, tools: &[&str], woken: Option<bool>) -> Vec<PromptFragment> {
        let contributor = self
            .prompts
            .registry()
            .snapshot()
            .items()
            .next()
            .cloned()
            .unwrap();
        let step = PromptStep {
            session_id: AgentSessionId::new(1),
            turn_id: AgentTurnId::new(2),
            generation: AgentGeneration::new(1),
            sampling_step: 1,
            tools: Arc::new(tools.iter().map(|tool| tool.to_string()).collect()),
        };
        let prepared = contributor
            .prepare(&step, CancellationToken::default())
            .await
            .unwrap();
        let mut turn = TurnExtensions::default();
        if let Some(on) = woken {
            turn.insert(Arc::new(TurnWake { on }));
        }
        contributor.contribute(&PromptRequest {
            turn: &turn,
            prepared: Some(&prepared),
            context_window_tokens: None,
        })
    }
}

/// The guidance on background terminals is shown to a step that offers
/// `exec_command` of a turn that is woken, as a section among the
/// capabilities, and states the command timeout the options set; a change
/// of them shows from the next step. Without the tool, or with the session
/// not woken, there is none.
#[tokio::test]
async fn the_background_terminals_guidance_states_the_command_timeout() {
    let mounted = mount_with(json!({ "commandTimeoutMinutes": 25 })).await;
    let renderer = mounted
        .renderers
        .registry()
        .snapshot()
        .items()
        .next()
        .cloned()
        .unwrap();
    assert_eq!(renderer.kind.as_str(), "background_terminals");
    assert_eq!(renderer.slot, ContextSlot::Capabilities);

    let shown = mounted
        .guidance(&["exec_command", "write_stdin"], Some(true))
        .await;
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].kind.as_str(), "background_terminals");
    let content = &shown[0].content;
    assert!(
        content.starts_with("<background_terminals>\n## Background terminals\n"),
        "{content}"
    );
    assert!(
        content.contains("times out (after 25 minutes)"),
        "{content}"
    );
    assert!(
        content.contains("you are resumed in a new turn"),
        "{content}"
    );

    assert!(mounted
        .guidance(&["write_stdin"], Some(true))
        .await
        .is_empty());
    assert!(mounted
        .guidance(&["exec_command"], Some(false))
        .await
        .is_empty());
    assert!(mounted.guidance(&["exec_command"], None).await.is_empty());

    mounted
        .fiber
        .update(json!({ "commandTimeoutMinutes": 90 }))
        .unwrap()
        .await
        .unwrap();
    let changed = mounted.guidance(&["exec_command"], Some(true)).await;
    assert!(
        changed[0].content.contains("times out (after 90 minutes)"),
        "{}",
        changed[0].content
    );
    mounted.root.fiber().dispose().await.unwrap();
}

/// A background terminal's end, here the command timeout, is reported
/// through the sink of the call that started it, after that call returned
/// its session and with no disposal: the kernel queues it on the session
/// as a wake event. A poll that then sees the end reports the disposal
/// through its own call's sink, after the end.
#[tokio::test(flavor = "multi_thread")]
async fn a_background_terminals_end_is_reported_through_its_starting_calls_sink() {
    let mounted = mount().await;
    let progress = Arc::new(RecordingProgress::default());
    mounted
        .run(command("serve", ToolCallOrigin::Model, progress.clone()))
        .await;
    let execution_id = progress
        .0
        .lock()
        .unwrap()
        .iter()
        .find_map(|update| match update {
            AgentToolProgressUpdate::ExecutionStarted { execution_id, .. } => Some(*execution_id),
            _ => None,
        })
        .unwrap();

    mounted.process.time_out();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !progress.0.lock().unwrap().iter().any(|update| {
        matches!(
            update,
            AgentToolProgressUpdate::ExecutionStatus {
                status: AgentExecutionProgressStatus::TimedOut,
                ..
            }
        )
    }) {
        assert!(Instant::now() < deadline, "the timeout was not reported");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!progress
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|update| matches!(update, AgentToolProgressUpdate::ExecutionDisposed { .. })));

    let polled = Arc::new(RecordingProgress::default());
    let mut poll = command("", ToolCallOrigin::Model, polled.clone());
    poll.arguments = json!({ "session_id": execution_id, "chars": "", "yield_time_ms": 1 });
    let write_stdin = mounted
        .tools
        .registry()
        .snapshot()
        .items()
        .find(|tool| tool.descriptor().name.qualified() == "write_stdin")
        .cloned()
        .unwrap();
    write_stdin.invoke(poll, None).await.unwrap();
    assert!(matches!(
        &polled.0.lock().unwrap()[..],
        [AgentToolProgressUpdate::ExecutionDisposed { execution_id: id, .. }] if *id == execution_id
    ));
    mounted.root.fiber().dispose().await.unwrap();
}
