//! Registered fake tools for kernel contract tests.
//!
//! The application's real tools live in plugins the kernel cannot depend
//! on, so kernel tests register fixtures under the same names: reads that
//! need no approval and mutations that ask for approval of a domain action
//! derived from their arguments.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, AuthorizedAction,
    PermissionRequirement,
};
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentToolError, AgentToolHandler, Register,
    ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolRegistry, ToolResult,
};
use hanafubuki_agent_model_api::ModelToolSpec;
use hanafubuki_agent_tool_contracts::{ToolExecutionClass, ToolExposure, ToolName};
use kardia_core::{Context, Fiber, Inject, PluginDef, Service};
use serde_json::{json, Value};
use tokio::sync::Barrier;

use crate::{kernel_plugin, AgentKernelRegistries};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FakeKind {
    /// No approval; concurrent.
    Read,
    /// No approval; serialized with mutations.
    ExclusiveRead,
    /// Approval of a domain action derived from the arguments.
    Mutation,
    /// A mutation that only `tool_search` admits.
    DeferredMutation,
}

impl FakeKind {
    fn is_mutation(self) -> bool {
        matches!(self, Self::Mutation | Self::DeferredMutation)
    }
}

/// The fixtures every [`FakeTools`] registers, so every test sees one
/// catalog.
const STANDARD_FAKE_TOOLS: [(&str, FakeKind); 8] = [
    ("list_instances", FakeKind::Read),
    ("list_tasks", FakeKind::Read),
    ("read_task_log", FakeKind::Read),
    ("fetch_instance_remote_catalog", FakeKind::ExclusiveRead),
    ("control_instance", FakeKind::Mutation),
    ("manage_instance_extension", FakeKind::Mutation),
    ("configure_instance", FakeKind::DeferredMutation),
    ("provision_instance", FakeKind::Mutation),
];

/// The model-visible specs of the directly offered standard fixtures, as
/// the kernel plans them (before provider projection).
pub fn standard_fake_tool_specs() -> Vec<ModelToolSpec> {
    STANDARD_FAKE_TOOLS
        .iter()
        .filter(|(_, kind)| *kind != FakeKind::DeferredMutation)
        .map(|(name, _)| ModelToolSpec {
            name: (*name).to_string(),
            description: format!("{name} fixture"),
            input_schema: fixture_schema(),
            strict: false,
        })
        .collect()
}

fn fixture_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": true })
}

/// The approval action a mutation fixture asks for: one domain mutation per
/// exact tool name and argument object.
pub fn fake_mutation_action(tool: &str, arguments: &Value) -> ApprovalAction {
    let mut canonical = format!("fake-domain-mutation-v1\0{tool}\0").into_bytes();
    canonical.extend(
        serde_json::to_vec(&hanafubuki_agent_tool_contracts::canonical_json(arguments))
            .expect("fixture arguments serialize"),
    );
    let summary = match (
        tool,
        arguments["action"].as_str(),
        arguments["instanceId"].as_str(),
    ) {
        ("control_instance", Some(action), Some(instance)) => {
            let mut verb = action.to_string();
            if let Some(first) = verb.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            format!("{verb} instance {instance}")
        }
        _ => format!("Run {tool}"),
    };
    ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::DomainMutation,
        ApprovalRiskClass::High,
        ApprovalOrigin::Model,
        summary,
        &canonical,
    )
    .expect("fixture approval action is valid")
}

/// One expected call of a registered fixture and its scripted result.
#[derive(Clone, Debug)]
pub struct FakeRegisteredExchange {
    pub tool: String,
    pub arguments: Value,
    pub output: Result<ToolResult, AgentToolError>,
}

impl FakeRegisteredExchange {
    pub fn success(tool: &str, arguments: Value, output: impl Into<ToolResult>) -> Self {
        Self {
            tool: tool.to_string(),
            arguments,
            output: Ok(output.into()),
        }
    }

    pub fn failure(tool: &str, arguments: Value, error: AgentToolError) -> Self {
        Self {
            tool: tool.to_string(),
            arguments,
            output: Err(error),
        }
    }
}

/// Two barriers a registered call waits on before it runs: the first tells
/// the test the call is in flight, the second lets it continue.
#[derive(Clone)]
struct InvokeGate {
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

#[derive(Default)]
struct FakeState {
    exchanges: VecDeque<FakeRegisteredExchange>,
    calls: Vec<(String, Value)>,
    approved_calls: usize,
    gate: Option<InvokeGate>,
    forbidden: Option<String>,
}

/// Kernel registries with the standard registered fixtures, mounted on
/// their own kardia root; hand [`FakeTools::registries`] to the service.
#[derive(Clone)]
pub struct FakeTools {
    state: Arc<Mutex<FakeState>>,
    registries: AgentKernelRegistries,
    root: Context,
}

impl FakeTools {
    pub async fn new(exchanges: impl IntoIterator<Item = FakeRegisteredExchange>) -> Self {
        let state = Arc::new(Mutex::new(FakeState {
            exchanges: exchanges.into_iter().collect(),
            ..FakeState::default()
        }));
        let registries = AgentKernelRegistries::default();
        let root = Context::new();
        root.plugin_def(&kernel_plugin(registries.clone()), Value::Null)
            .expect("kernel plugin mounts")
            .settled()
            .await
            .expect("kernel plugin starts");
        let registered = state.clone();
        let plugin = PluginDef::new(move |ctx: Context, _| {
            let state = registered.clone();
            async move {
                let tools = ctx.get::<ToolRegistry>()?;
                for (name, kind) in STANDARD_FAKE_TOOLS {
                    tools.register(Arc::new(FakeHandler::new(name, kind, state.clone())))?;
                }
                Ok(())
            }
        })
        .name("fake-tools")
        .inject(Inject::new().with(ToolRegistry::NAME));
        root.plugin_def(&plugin, Value::Null)
            .expect("fake tools mount")
            .settled()
            .await
            .expect("fake tools start");
        Self {
            state,
            registries,
            root,
        }
    }

    /// The registries the kernel plans from.
    pub fn registries(&self) -> AgentKernelRegistries {
        self.registries.clone()
    }

    /// Mounts another plugin beside the standard fixtures; dispose the
    /// returned fiber to stop it.
    pub async fn mount(&self, plugin: &PluginDef) -> Fiber {
        let fiber = self
            .root
            .plugin_def(plugin, Value::Null)
            .expect("fixture plugin mounts");
        fiber.settled().await.expect("fixture plugin starts");
        fiber
    }

    /// Makes every mutation fixture's permission `Forbidden` for `reason`.
    pub fn with_forbidden_mutations(self, reason: &str) -> Self {
        self.state.lock().unwrap().forbidden = Some(reason.to_string());
        self
    }

    /// Makes every registered call wait on `entered`, then on `release`,
    /// before it runs.
    pub fn with_invoke_gate(self, entered: Arc<Barrier>, release: Arc<Barrier>) -> Self {
        self.state.lock().unwrap().gate = Some(InvokeGate { entered, release });
        self
    }

    /// Registered calls in order, by tool name and arguments.
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn approved_calls(&self) -> usize {
        self.state.lock().unwrap().approved_calls
    }

    pub fn remaining_exchanges(&self) -> usize {
        self.state.lock().unwrap().exchanges.len()
    }
}

struct FakeHandler {
    kind: FakeKind,
    descriptor: ToolDescriptor,
    state: Arc<Mutex<FakeState>>,
}

impl FakeHandler {
    fn new(name: &str, kind: FakeKind, state: Arc<Mutex<FakeState>>) -> Self {
        Self {
            kind,
            descriptor: ToolDescriptor {
                name: ToolName::parse(name).expect("fixture tool name"),
                description: format!("{name} fixture"),
                input_schema: fixture_schema(),
                exposure: match kind {
                    FakeKind::DeferredMutation => ToolExposure::Deferred,
                    FakeKind::Read | FakeKind::ExclusiveRead | FakeKind::Mutation => {
                        ToolExposure::Direct
                    }
                },
                execution_class: match kind {
                    FakeKind::Read => ToolExecutionClass::ConcurrentRead,
                    FakeKind::ExclusiveRead | FakeKind::Mutation | FakeKind::DeferredMutation => {
                        ToolExecutionClass::Exclusive
                    }
                },
                effect: if kind.is_mutation() {
                    ToolEffect::Mutating
                } else {
                    ToolEffect::ReadOnly
                },
                action: AgentActivityAction::parse(name).expect("fixture action name"),
                memory_effect: AgentMemoryContextEffect::None,
                timeout: None,
            },
            state,
        }
    }

    fn name(&self) -> String {
        self.descriptor.name.qualified()
    }
}

impl AgentToolHandler for FakeHandler {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        let forbidden = self.state.lock().unwrap().forbidden.clone();
        let requirement = match (self.kind.is_mutation(), forbidden) {
            (false, _) => PermissionRequirement::NotRequired,
            (true, Some(reason)) => PermissionRequirement::Forbidden { reason },
            (true, None) => PermissionRequirement::ReviewRequired(fake_mutation_action(
                &self.name(),
                &invocation.arguments,
            )),
        };
        let cancelled = invocation.cancellation.is_cancelled();
        Box::pin(async move {
            if cancelled {
                Err(AgentToolError::Cancelled)
            } else {
                Ok(requirement)
            }
        })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        let gate = self.state.lock().unwrap().gate.clone();
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.entered.wait().await;
                gate.release.wait().await;
            }
            self.run(invocation, approval)
        })
    }
}

impl FakeHandler {
    fn run(
        &self,
        invocation: ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> Result<ToolResult, AgentToolError> {
        (|| {
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            let name = self.name();
            if self.kind.is_mutation() {
                let expected = fake_mutation_action(&name, &invocation.arguments);
                if approval.as_ref().map(AuthorizedAction::action) != Some(&expected) {
                    return Err(AgentToolError::Failed(
                        "fixture mutation ran without its exact approval".to_string(),
                    ));
                }
            }
            let mut state = self.state.lock().unwrap();
            if approval.is_some() {
                state.approved_calls += 1;
            }
            state
                .calls
                .push((name.clone(), invocation.arguments.clone()));
            match state.exchanges.pop_front() {
                Some(exchange)
                    if exchange.tool == name && exchange.arguments == invocation.arguments =>
                {
                    exchange.output
                }
                Some(exchange) => {
                    state.exchanges.push_front(exchange);
                    Err(AgentToolError::Failed(
                        "fake tool call did not match the next deterministic fixture".to_string(),
                    ))
                }
                None => Err(AgentToolError::Failed(
                    "fake tool port has no remaining exchange".to_string(),
                )),
            }
        })()
    }
}
