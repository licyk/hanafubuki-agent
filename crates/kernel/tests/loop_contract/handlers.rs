//! Plugin-registered tools run through the kernel's fixed pipeline.

use std::sync::atomic::AtomicBool;

use hanafubuki_agent_approval::PermissionRequirement;
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentToolHandler, Register, RegistryService,
    ToolContextFragment, ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolRegistry,
    ToolResult,
};
use hanafubuki_agent_kernel::{kernel_plugin, AgentKernelRegistries};
use hanafubuki_agent_tool_contracts::{ToolExecutionClass, ToolExposure, ToolName};
use kardia_core::{Context, Fiber, Inject, PluginDef, Service};
use serde_json::Value;

use super::*;

enum Behaviour {
    Echo,
    WaitForCancellation(Arc<AtomicBool>),
    /// Never finishes; keeps the call's cancellation for the test to read.
    Hang(Arc<Mutex<Option<CancellationToken>>>),
    /// Reports the turn's Skill catalog revision and returns the context
    /// fragment its `source` argument names.
    Context,
}

struct FakeTool {
    descriptor: ToolDescriptor,
    behaviour: Behaviour,
}

impl FakeTool {
    fn new(name: &str, effect: ToolEffect, behaviour: Behaviour) -> Arc<dyn AgentToolHandler> {
        Arc::new(Self {
            descriptor: ToolDescriptor {
                name: ToolName::parse(name).unwrap(),
                description: format!("{name} fixture"),
                input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": true }),
                exposure: ToolExposure::Direct,
                execution_class: match effect {
                    ToolEffect::ReadOnly => ToolExecutionClass::ConcurrentRead,
                    ToolEffect::Mutating => ToolExecutionClass::Exclusive,
                },
                effect,
                action: AgentActivityAction::USE_TOOL,
                memory_effect: AgentMemoryContextEffect::None,
                timeout: None,
            },
            behaviour,
        })
    }
}

impl AgentToolHandler for FakeTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        _invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async { Ok(PermissionRequirement::NotRequired) })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        _approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            match &self.behaviour {
                Behaviour::Echo => Ok(ToolResult::success(json!({ "echo": invocation.arguments }))),
                Behaviour::Context => {
                    let selectable = invocation
                        .turn
                        .get::<hanafubuki_agent_contracts::SelectableSkills>()
                        .map(|selectable| selectable.skills.len());
                    let source = invocation.arguments["source"].as_str().unwrap_or_default();
                    Ok(
                        ToolResult::success(json!({ "selectableSkills": selectable }))
                            .with_context(ToolContextFragment {
                                source: source.to_string(),
                                name: "Workflow".to_string(),
                                content: "<skill>\nFollow the workflow.\n</skill>".to_string(),
                            }),
                    )
                }
                Behaviour::Hang(cancellation) => {
                    *cancellation.lock().unwrap() = Some(invocation.cancellation.clone());
                    std::future::pending().await
                }
                Behaviour::WaitForCancellation(started) => {
                    started.store(true, Ordering::Release);
                    while !invocation.cancellation.is_cancelled() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    Err(AgentToolError::Cancelled)
                }
            }
        })
    }
}

struct Fixture {
    root: Context,
    plugin: Fiber,
    tools: Arc<ToolRegistry>,
    waiting_started: Arc<AtomicBool>,
    hanging_call: Arc<Mutex<Option<CancellationToken>>>,
}

async fn fixture() -> Fixture {
    let registries = AgentKernelRegistries::default();
    let tools = registries.tools.clone();
    let root = Context::new();
    root.plugin_def(&kernel_plugin(registries), Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    let waiting_started = Arc::new(AtomicBool::new(false));
    let started = waiting_started.clone();
    let hanging_call = Arc::new(Mutex::new(None));
    let hanging = hanging_call.clone();
    let plugin = PluginDef::new(move |ctx: Context, _| {
        let started = started.clone();
        let hanging = hanging.clone();
        async move {
            let tools = ctx.get::<ToolRegistry>()?;
            tools.register(FakeTool::new(
                "fake_read",
                ToolEffect::ReadOnly,
                Behaviour::Echo,
            ))?;
            tools.register(FakeTool::new(
                "fake_write",
                ToolEffect::Mutating,
                Behaviour::Echo,
            ))?;
            tools.register(FakeTool::new(
                "fake_skill",
                ToolEffect::ReadOnly,
                Behaviour::Context,
            ))?;
            tools.register(FakeTool::new(
                "fake_wait",
                ToolEffect::ReadOnly,
                Behaviour::WaitForCancellation(started),
            ))?;
            let mut hang = FakeTool {
                descriptor: FakeTool::new("fake_hang", ToolEffect::ReadOnly, Behaviour::Echo)
                    .descriptor()
                    .clone(),
                behaviour: Behaviour::Hang(hanging),
            };
            hang.descriptor.timeout = Some(HANG_TIMEOUT);
            tools.register(Arc::new(hang))?;
            Ok(())
        }
    })
    .name("fake-tools")
    .inject(Inject::new().with(ToolRegistry::NAME));
    let plugin = root.plugin_def(&plugin, Value::Null).unwrap();
    plugin.settled().await.unwrap();
    Fixture {
        root,
        plugin,
        tools,
        waiting_started,
        hanging_call,
    }
}

/// The descriptor timeout of `fake_hang`.
const HANG_TIMEOUT: Duration = Duration::from_millis(50);

fn tool_call_events(call_id: &str, name: &str, arguments: Value) -> Vec<ModelResult<ModelEvent>> {
    let call = ModelToolCall {
        item_id: format!("item-{call_id}"),
        call_id: call_id.to_string(),
        namespace: None,
        name: name.to_string(),
        arguments,
        argument_error: None,
    };
    vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: format!("response-{call_id}"),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall { call: call.clone() },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { call },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: format!("response-{call_id}"),
            usage: None,
            end_turn: Some(false),
        }),
    ]
}

fn offered_tools(request: &ModelRequest) -> String {
    format!("{:?}", request.tools)
}

#[tokio::test]
async fn a_registered_tool_runs_and_its_output_reaches_the_next_request() {
    let fixture = fixture().await;
    let model = ScriptedEventModel::new([
        tool_call_events("call-read", "fake_read", json!({ "path": "a" })),
        deterministic_assistant_events("done"),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        AgentKernelRegistries {
            tools: fixture.tools.clone(),
            ..AgentKernelRegistries::default()
        },
    );
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "read it".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "done");
    let requests = model.requests();
    assert!(offered_tools(&requests[0]).contains("\"fake_write\""));
    let second = format!("{:?}", requests[1].input);
    assert!(second.contains("echo"), "{second}");
    assert!(second.contains("\"path\": String(\"a\")"), "{second}");
    fixture.root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn plan_mode_offers_only_read_only_registered_tools() {
    let fixture = fixture().await;
    let model = ScriptedEventModel::new([deterministic_assistant_events("planned")]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        AgentKernelRegistries {
            tools: fixture.tools.clone(),
            ..AgentKernelRegistries::default()
        },
    );
    let mut plan_config = config();
    plan_config.mode = hanafubuki_agent_kernel::AgentSessionMode::Plan;
    let session = service.create_session(plan_config).unwrap();

    service
        .start_turn(session.id, "plan it".to_string())
        .await
        .unwrap();

    let requests = model.requests();
    let offered = offered_tools(&requests[0]);
    assert!(offered.contains("\"fake_read\""));
    assert!(!offered.contains("\"fake_write\""));
    fixture.root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn a_call_outliving_its_stopping_plugin_is_cancelled_and_reported() {
    let fixture = fixture().await;
    let model = ScriptedEventModel::new([
        tool_call_events("call-wait", "fake_wait", json!({})),
        deterministic_assistant_events("recovered"),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        AgentKernelRegistries {
            tools: fixture.tools.clone(),
            ..AgentKernelRegistries::default()
        },
    );
    let session = service.create_session(config()).unwrap();
    let executing = service.clone();
    let session_id = session.id;
    let turn =
        tokio::spawn(async move { executing.start_turn(session_id, "wait".to_string()).await });
    while !fixture.waiting_started.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    fixture.plugin.dispose().await.unwrap();

    assert!(fixture.tools.registry().snapshot().is_empty());
    let outcome = turn.await.unwrap().unwrap();
    assert_eq!(outcome.final_answer, "recovered");
    let second = format!("{:?}", model.requests()[1].input);
    assert!(
        second.contains("tool was stopped because its plugin stopped"),
        "{second}"
    );
    fixture.root.fiber().dispose().await.unwrap();
}

/// A registered tool reads the turn's frozen Skill catalog and adopts a
/// Skill by returning its context: the kernel commits it once per turn as
/// that Skill's context, and refuses a fragment naming no Skill.
#[tokio::test]
async fn turn_data_reaches_tools_and_context_fragments_commit_once_as_skill_context() {
    let fixture = fixture().await;
    let model = ScriptedEventModel::new([
        tool_call_events(
            "call-first",
            "fake_skill",
            json!({ "source": "user:workflow" }),
        ),
        tool_call_events(
            "call-again",
            "fake_skill",
            json!({ "source": "user:workflow" }),
        ),
        tool_call_events(
            "call-invalid",
            "fake_skill",
            json!({ "source": "not a skill" }),
        ),
        deterministic_assistant_events("adopted"),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        AgentKernelRegistries {
            tools: fixture.tools.clone(),
            ..AgentKernelRegistries::default()
        },
    );
    let session = service.create_session(config()).unwrap();
    let mut input = AgentTurnInput::from("adopt it".to_string());
    input.turn = selecting(selectable_skills_fixture());

    let outcome = service.start_turn(session.id, input).await.unwrap();

    assert_eq!(outcome.final_answer, "adopted");
    let context = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    let skill_contexts = context
        .iter()
        .filter_map(|item| match item {
            ConversationItem::SkillContext {
                skill,
                name,
                content,
            } => Some((skill.as_str().to_string(), name.clone(), content.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        skill_contexts,
        vec![(
            "user:workflow".to_string(),
            "Workflow".to_string(),
            "<skill>\nFollow the workflow.\n</skill>".to_string()
        )]
    );
    let outputs = context
        .iter()
        .filter_map(|item| match item {
            ConversationItem::ToolOutput { output, .. } => Some(output.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(outputs.len(), 3);
    assert_eq!(
        outputs[0].structured_content,
        Some(json!({ "selectableSkills": 1 }))
    );
    assert!(outputs[2].is_error);
    assert!(format!("{:?}", outputs[2]).contains("invalid Skill id"));
    fixture.root.fiber().dispose().await.unwrap();
}

/// A call that runs past its descriptor's timeout is cancelled and answered
/// with a timeout error; the step's other call and the turn carry on.
#[tokio::test]
async fn a_call_past_its_timeout_is_cancelled_and_the_turn_continues() {
    let fixture = fixture().await;
    let model = ScriptedEventModel::new([
        calls_events(&[
            ("call-hang", "fake_hang", json!({})),
            ("call-read", "fake_read", json!({ "path": "a" })),
        ]),
        deterministic_assistant_events("recovered"),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        AgentKernelRegistries {
            tools: fixture.tools.clone(),
            ..AgentKernelRegistries::default()
        },
    );
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "hang".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "recovered");
    let cancellation = fixture.hanging_call.lock().unwrap().clone().unwrap();
    assert!(cancellation.is_cancelled());
    let outputs = test_session_view(&service, session.id)
        .unwrap()
        .model_context
        .into_iter()
        .filter_map(|item| match item {
            ConversationItem::ToolOutput {
                call_id, output, ..
            } => Some((call_id, output)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0].0, "call-hang");
    assert!(outputs[0].1.is_error);
    assert_eq!(
        outputs[0].1.structured_content,
        Some(json!({ "error": "the tool did not finish within 0.05 s and was stopped" }))
    );
    assert_eq!(outputs[1].0, "call-read");
    assert!(!outputs[1].1.is_error);
    fixture.root.fiber().dispose().await.unwrap();
}
