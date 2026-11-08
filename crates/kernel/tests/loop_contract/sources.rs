//! Tools a source plans per step run through the same pipeline as registered
//! tools, with the source's plan lease, a schema check of the model's
//! arguments before `permission`, and a lease on the source's entry.

use hanafubuki_agent_approval::PermissionRequirement;
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentToolHandler, AgentToolSource, SourcedTool,
    ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolResult, ToolSourceContribution,
    ToolSourceRegistry, ToolSourceStep, ToolSourceTrust,
};
use hanafubuki_agent_kernel::{kernel_plugin, AgentKernelRegistries};
use hanafubuki_agent_tool_contracts::{
    ToolExecutionClass, ToolExposure, ToolGeneration, ToolId, ToolName, ToolProviderId,
};
use kardia_core::{Fiber, Service};
use serde_json::Value;

use super::*;

struct SourcedFake {
    descriptor: ToolDescriptor,
    permissions: Arc<AtomicUsize>,
    started: Arc<AtomicBool>,
    waits: bool,
}

impl AgentToolHandler for SourcedFake {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        _invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        self.permissions.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(PermissionRequirement::NotRequired) })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        _approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            if self.waits {
                self.started.store(true, Ordering::Release);
                while !invocation.cancellation.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                return Err(AgentToolError::Cancelled);
            }
            let lease = invocation.plan_lease::<String>().cloned();
            Ok(ToolResult::success(
                json!({ "echo": invocation.arguments, "lease": lease }),
            ))
        })
    }
}

/// Plans `sourced_read` (read-only, `name` of at most 4 characters),
/// `sourced_write` and `sourced_wait` with the lease `"step-lease"`.
struct FakeSource {
    permissions: Arc<AtomicUsize>,
    started: Arc<AtomicBool>,
}

impl FakeSource {
    fn tool(&self, name: &str, effect: ToolEffect, waits: bool) -> SourcedTool {
        SourcedTool {
            handler: Arc::new(SourcedFake {
                descriptor: ToolDescriptor {
                    name: ToolName::namespaced(vec!["fake".to_string()], name).unwrap(),
                    description: format!("{name} fixture"),
                    input_schema: json!({
                        "type": "object",
                        "properties": {"name": {"type": "string", "maxLength": 4}},
                        "additionalProperties": false
                    }),
                    exposure: ToolExposure::Direct,
                    execution_class: ToolExecutionClass::Exclusive,
                    effect,
                    action: AgentActivityAction::USE_TOOL,
                    memory_effect: AgentMemoryContextEffect::None,
                    timeout: None,
                },
                permissions: self.permissions.clone(),
                started: self.started.clone(),
                waits,
            }),
            provider_id: ToolProviderId::parse("fake.source").unwrap(),
            tool_id: ToolId::parse(name).unwrap(),
            generation: ToolGeneration([name.len() as u8; 32]),
            namespace_description: None,
            subject: Some("fake source".to_string()),
            trust: ToolSourceTrust::Optional,
        }
    }
}

impl AgentToolSource for FakeSource {
    fn id(&self) -> &str {
        "fake-source"
    }

    fn contribute<'a>(
        &'a self,
        _step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>> {
        Box::pin(async move {
            Ok(ToolSourceContribution::from_tools(vec![
                self.tool("sourced_read", ToolEffect::ReadOnly, false),
                self.tool("sourced_write", ToolEffect::Mutating, false),
                self.tool("sourced_wait", ToolEffect::ReadOnly, true),
            ])
            .with_lease(Arc::new("step-lease".to_string())))
        })
    }
}

struct Fixture {
    root: Context,
    plugin: Fiber,
    registries: AgentKernelRegistries,
    permissions: Arc<AtomicUsize>,
    started: Arc<AtomicBool>,
}

async fn fixture() -> Fixture {
    let registries = AgentKernelRegistries::default();
    let root = Context::new();
    root.plugin_def(&kernel_plugin(registries.clone()), Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    let permissions = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(AtomicBool::new(false));
    let source = Arc::new(FakeSource {
        permissions: permissions.clone(),
        started: started.clone(),
    });
    let plugin = PluginDef::new(move |ctx: Context, _| {
        let source = source.clone();
        async move {
            ctx.get::<ToolSourceRegistry>()?.register(source)?;
            Ok(())
        }
    })
    .name("fake-source")
    .inject(Inject::new().with(ToolSourceRegistry::NAME));
    let plugin = root.plugin_def(&plugin, Value::Null).unwrap();
    plugin.settled().await.unwrap();
    Fixture {
        root,
        plugin,
        registries,
        permissions,
        started,
    }
}

fn call_events(call_id: &str, name: &str, arguments: Value) -> Vec<ModelResult<ModelEvent>> {
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

#[tokio::test]
async fn sourced_arguments_are_checked_before_permission_and_calls_carry_the_plan_lease() {
    let fixture = fixture().await;
    let model = ScriptedEventModel::new([
        call_events(
            "call-invalid",
            "fake__sourced_read",
            json!({ "name": "too-long" }),
        ),
        call_events("call-valid", "fake__sourced_read", json!({ "name": "ok" })),
        deterministic_assistant_events("done"),
    ]);
    let service = AgentService::new(Arc::new(model.clone()), fixture.registries.clone());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "read it".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "done");
    // Only the valid call reached `permission`.
    assert_eq!(fixture.permissions.load(Ordering::SeqCst), 1);
    let requests = model.requests();
    let rejected = format!("{:?}", requests[1].input);
    assert!(
        rejected.contains("dynamic tool arguments are invalid"),
        "{rejected}"
    );
    let answered = format!("{:?}", requests[2].input);
    assert!(answered.contains("step-lease"), "{answered}");
    fixture.root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn plan_mode_offers_only_read_only_sourced_tools() {
    let fixture = fixture().await;
    let model = ScriptedEventModel::new([deterministic_assistant_events("planned")]);
    let service = AgentService::new(Arc::new(model.clone()), fixture.registries.clone());
    let mut plan_config = config();
    plan_config.mode = hanafubuki_agent_kernel::AgentSessionMode::Plan;
    let session = service.create_session(plan_config).unwrap();

    service
        .start_turn(session.id, "plan it".to_string())
        .await
        .unwrap();

    let offered = format!("{:?}", model.requests()[0].tools);
    assert!(offered.contains("sourced_read"), "{offered}");
    assert!(!offered.contains("sourced_write"), "{offered}");
    fixture.root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn a_sourced_call_outliving_its_stopping_source_is_cancelled_and_reported() {
    let fixture = fixture().await;
    let model = ScriptedEventModel::new([
        call_events("call-wait", "fake__sourced_wait", json!({})),
        deterministic_assistant_events("recovered"),
    ]);
    let service = AgentService::new(Arc::new(model.clone()), fixture.registries.clone());
    let session = service.create_session(config()).unwrap();
    let executing = service.clone();
    let session_id = session.id;
    let turn =
        tokio::spawn(async move { executing.start_turn(session_id, "wait".to_string()).await });
    while !fixture.started.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    fixture.plugin.dispose().await.unwrap();

    assert!(fixture
        .registries
        .tool_sources
        .registry()
        .snapshot()
        .is_empty());
    let outcome = turn.await.unwrap().unwrap();
    assert_eq!(outcome.final_answer, "recovered");
    let second = format!("{:?}", model.requests()[1].input);
    assert!(
        second.contains("tool was stopped because its plugin stopped"),
        "{second}"
    );
    fixture.root.fiber().dispose().await.unwrap();
}
