//! Every kernel call into a plugin that has no bound of its own runs within
//! `AgentHardLimits::plugin_call_deadline`: a call that never answers is
//! cancelled, its contribution is left out (a tool's `permission` fails the
//! call) and the turn goes on.

use hanafubuki_agent_approval::PermissionRequirement;
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentToolHandler, AgentToolSource,
    ContextContributor, ContextFuture, ContextRegistry, ContextRequest, PromptPrepareFuture,
    PromptStep, ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolRegistry, ToolResult,
    ToolSourceContribution, ToolSourceRegistry, ToolSourceStep, ToolSourceTrust,
    TurnAdmissionContributor, TurnAdmissionFuture, TurnAdmissionRegistry, TurnAdmissionRequest,
};
use hanafubuki_agent_kernel::{
    collect_turn_extensions, collect_world_state, AgentKernelRegistries, AgentPermissionMode,
    AgentSessionMode,
};
use hanafubuki_agent_tool_contracts::{ToolExecutionClass, ToolExposure, ToolName};

use super::*;

const DEADLINE: Duration = Duration::from_millis(100);

/// What a [`Hung`] call was given to observe.
type Seen = Arc<Mutex<Option<CancellationToken>>>;

/// A plugin whose every call keeps the token it was given and never
/// answers, whatever the token says.
struct Hung {
    id: &'static str,
    seen: Seen,
    descriptor: ToolDescriptor,
    trust: ToolSourceTrust,
}

impl Hung {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            seen: Seen::default(),
            descriptor: ToolDescriptor {
                name: ToolName::parse("hung_permission").unwrap(),
                description: "hung_permission fixture".to_string(),
                input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
                exposure: ToolExposure::Direct,
                execution_class: ToolExecutionClass::ConcurrentRead,
                effect: ToolEffect::ReadOnly,
                action: AgentActivityAction::USE_TOOL,
                memory_effect: AgentMemoryContextEffect::None,
                timeout: None,
            },
            trust: ToolSourceTrust::Optional,
        }
    }

    fn hang<T: Send + 'static>(
        &self,
        token: CancellationToken,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'static>> {
        *self.seen.lock().unwrap() = Some(token);
        Box::pin(std::future::pending())
    }

    /// Whether the token of its call was cancelled.
    fn cancelled(seen: &Seen) -> bool {
        seen.lock()
            .unwrap()
            .as_ref()
            .expect("the plugin was called")
            .is_cancelled()
    }
}

impl ContextContributor for Hung {
    fn id(&self) -> &str {
        self.id
    }

    fn kinds(&self) -> &[ContextSectionKind] {
        const KINDS: &[ContextSectionKind] = &[HOST_LOCATION];
        KINDS
    }

    fn contribute<'a>(
        &'a self,
        _request: &'a ContextRequest,
        cancellation: CancellationToken,
    ) -> ContextFuture<'a> {
        self.hang(cancellation)
    }
}

impl TurnAdmissionContributor for Hung {
    fn id(&self) -> &str {
        self.id
    }

    fn rank(&self) -> u32 {
        0
    }

    fn contribute<'a>(
        &'a self,
        _request: &'a TurnAdmissionRequest,
        cancellation: CancellationToken,
    ) -> TurnAdmissionFuture<'a> {
        self.hang(cancellation)
    }
}

impl PromptContributor for Hung {
    fn id(&self) -> &str {
        self.id
    }

    fn rank(&self) -> u32 {
        0
    }

    fn prepare<'a>(
        &'a self,
        _step: &'a PromptStep,
        cancellation: CancellationToken,
    ) -> PromptPrepareFuture<'a> {
        self.hang(cancellation)
    }

    fn contribute(&self, _request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        Vec::new()
    }
}

impl AgentToolSource for Hung {
    fn id(&self) -> &str {
        self.id
    }

    fn trust(&self) -> ToolSourceTrust {
        self.trust
    }

    fn contribute<'a>(
        &'a self,
        step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>> {
        self.hang(step.cancellation.clone())
    }
}

impl AgentToolHandler for Hung {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        self.hang(invocation.cancellation.clone())
    }

    fn invoke(
        &self,
        _invocation: ToolInvocation,
        _approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async { Ok(ToolResult::success(json!({ "ran": true }))) })
    }
}

fn limits() -> AgentHardLimits {
    AgentHardLimits {
        plugin_call_deadline: DEADLINE,
        ..FIRST_BATCH_HARD_LIMITS
    }
}

fn context_request() -> ContextRequest {
    ContextRequest {
        session_id: AgentSessionId::new(1),
        mode: AgentSessionMode::Default,
        permission_mode: AgentPermissionMode::Manual,
        user_message: None,
        turn: TurnExtensions::default(),
    }
}

fn admission_request() -> TurnAdmissionRequest {
    TurnAdmissionRequest {
        workspace: None,
        session_id: AgentSessionId::new(1),
        mode: AgentSessionMode::Default,
    }
}

/// The overrun as its plugin's status shows it.
fn overrun_issue(source: &str) -> Vec<String> {
    vec![format!(
        "{source}: it did not answer within {} s and was cancelled",
        DEADLINE.as_secs_f64()
    )]
}

/// Waits until the plugin that keeps `seen` is called.
async fn called(seen: &Seen) {
    while seen.lock().unwrap().is_none() {
        tokio::task::yield_now().await;
    }
}

/// Runs a turn answering "done" over `registries` and returns its answer.
async fn answered_turn(registries: AgentKernelRegistries) -> (String, ScriptedEventModel) {
    let model = ScriptedEventModel::new([deterministic_assistant_events("done")]);
    let service = AgentService::with_limits(Arc::new(model.clone()), registries, limits());
    let session = service.create_session(config()).unwrap();
    let outcome = service
        .start_turn(session.id, "go".to_string())
        .await
        .unwrap();
    (outcome.final_answer, model)
}

#[tokio::test]
async fn a_hung_context_contributor_is_left_out_with_an_issue() {
    let tools = FakeTools::new([]).await;
    let hung = Arc::new(Hung::new("hung-context"));
    let seen = hung.seen.clone();
    tools
        .mount(&registering::<ContextRegistry>(
            "hung-context",
            vec![hung as Arc<dyn ContextContributor>],
        ))
        .await;
    let registries = tools.registries();

    let world_state = collect_world_state(
        &registries.context,
        &context_request(),
        &hanafubuki_agent_kernel::AgentCapabilityScope::default(),
        &CancellationToken::default(),
        DEADLINE,
    )
    .await
    .unwrap();

    assert!(world_state.sections.is_empty());
    assert!(Hung::cancelled(&seen));
    assert_eq!(
        registries.faults.issues("hung-context"),
        overrun_issue("context contributor hung-context")
    );
}

#[tokio::test]
async fn a_hung_turn_admission_contributor_is_left_out_with_an_issue() {
    let tools = FakeTools::new([]).await;
    let hung = Arc::new(Hung::new("hung-admission"));
    let seen = hung.seen.clone();
    tools
        .mount(&registering::<TurnAdmissionRegistry>(
            "hung-admission",
            vec![hung as Arc<dyn TurnAdmissionContributor>],
        ))
        .await;
    let registries = tools.registries();

    collect_turn_extensions(
        &registries.turn_admission,
        &admission_request(),
        &hanafubuki_agent_kernel::AgentCapabilityScope::default(),
        &CancellationToken::default(),
        DEADLINE,
    )
    .await
    .unwrap();

    assert!(Hung::cancelled(&seen));
    assert_eq!(
        registries.faults.issues("hung-admission"),
        overrun_issue("turn admission contributor hung-admission")
    );
}

#[tokio::test]
async fn cancelling_the_admission_cancels_a_hung_context_or_admission_call() {
    let tools = FakeTools::new([]).await;
    let context = Arc::new(Hung::new("hung-context"));
    let admission = Arc::new(Hung::new("hung-admission"));
    let (context_seen, admission_seen) = (context.seen.clone(), admission.seen.clone());
    tools
        .mount(&registering::<ContextRegistry>(
            "hung-context",
            vec![context as Arc<dyn ContextContributor>],
        ))
        .await;
    tools
        .mount(&registering::<TurnAdmissionRegistry>(
            "hung-admission",
            vec![admission as Arc<dyn TurnAdmissionContributor>],
        ))
        .await;
    let registries = tools.registries();
    // Far past the test's own timeout: only the cancellation ends the calls.
    let deadline = Duration::from_secs(3600);

    let (context, admission) = (context_request(), admission_request());
    let scope = hanafubuki_agent_kernel::AgentCapabilityScope::default();
    let cancellation = CancellationToken::default();
    let (world_state, ()) = tokio::join!(
        collect_world_state(
            &registries.context,
            &context,
            &scope,
            &cancellation,
            deadline
        ),
        async {
            called(&context_seen).await;
            cancellation.cancel();
        },
    );
    assert!(matches!(world_state, Err(AgentError::Cancelled)));
    assert!(Hung::cancelled(&context_seen));

    let cancellation = CancellationToken::default();
    let (extensions, ()) = tokio::join!(
        collect_turn_extensions(
            &registries.turn_admission,
            &admission,
            &scope,
            &cancellation,
            deadline
        ),
        async {
            called(&admission_seen).await;
            cancellation.cancel();
        },
    );
    assert!(matches!(extensions, Err(AgentError::Cancelled)));
    assert!(Hung::cancelled(&admission_seen));
    assert!(registries.faults.all_issues().is_empty());
}

#[tokio::test]
async fn dropping_an_admission_cancels_its_call_in_flight() {
    let tools = FakeTools::new([]).await;
    let hung = Arc::new(Hung::new("hung-context"));
    let seen = hung.seen.clone();
    tools
        .mount(&registering::<ContextRegistry>(
            "hung-context",
            vec![hung as Arc<dyn ContextContributor>],
        ))
        .await;
    let registries = tools.registries();
    let request = context_request();
    let scope = hanafubuki_agent_kernel::AgentCapabilityScope::default();
    let never_cancelled = CancellationToken::default();

    tokio::select! {
        _ = collect_world_state(
            &registries.context,
            &request,
            &scope,
            &never_cancelled,
            Duration::from_secs(3600),
        ) => panic!("the hung call answered"),
        () = called(&seen) => {}
    }

    assert!(Hung::cancelled(&seen));
    assert!(!never_cancelled.is_cancelled());
}

#[tokio::test]
async fn a_hung_optional_tool_source_is_omitted_and_the_turn_completes() {
    let tools = FakeTools::new([]).await;
    let hung = Arc::new(Hung::new("hung-source"));
    let seen = hung.seen.clone();
    tools
        .mount(&registering::<ToolSourceRegistry>(
            "hung-source",
            vec![hung as Arc<dyn AgentToolSource>],
        ))
        .await;

    let (answer, model) = answered_turn(tools.registries()).await;

    assert_eq!(answer, "done");
    assert!(Hung::cancelled(&seen));
    let omissions = model.requests()[0]
        .input
        .iter()
        .find_map(|item| match item {
            ModelInputItem::ContextFragment { kind, content, .. }
                if kind == "tool_catalog_omissions" =>
            {
                Some(content.clone())
            }
            _ => None,
        })
        .expect("the omission is shown");
    assert!(
        omissions.contains("hung-source") && omissions.contains("did not answer within"),
        "{omissions}"
    );
}

#[tokio::test]
async fn a_hung_required_tool_source_fails_the_turn_rather_than_blocking_it() {
    let tools = FakeTools::new([]).await;
    let hung = Arc::new(Hung {
        trust: ToolSourceTrust::Required,
        ..Hung::new("hung-source")
    });
    tools
        .mount(&registering::<ToolSourceRegistry>(
            "hung-source",
            vec![hung as Arc<dyn AgentToolSource>],
        ))
        .await;
    let model = ScriptedEventModel::new([deterministic_assistant_events("done")]);
    let service = AgentService::with_limits(Arc::new(model), tools.registries(), limits());
    let session = service.create_session(config()).unwrap();

    let failed = service.start_turn(session.id, "go".to_string()).await;

    let error = failed.unwrap_err().to_string();
    assert!(error.contains("did not answer within"), "{error}");
}

#[tokio::test]
async fn a_hung_prompt_preparer_is_left_out_and_the_turn_completes() {
    let tools = FakeTools::new([]).await;
    let hung = Arc::new(Hung::new("hung-prompt"));
    let seen = hung.seen.clone();
    tools
        .mount(&registering::<PromptContributorRegistry>(
            "hung-prompt",
            vec![hung as Arc<dyn PromptContributor>],
        ))
        .await;

    let (answer, _) = answered_turn(tools.registries()).await;

    assert_eq!(answer, "done");
    assert!(Hung::cancelled(&seen));
}

#[tokio::test]
async fn a_hung_tool_permission_fails_the_call_and_the_turn_completes() {
    let tools = FakeTools::new([]).await;
    let hung = Arc::new(Hung::new("hung-permission"));
    let seen = hung.seen.clone();
    tools
        .mount(&registering::<ToolRegistry>(
            "hung-permission",
            vec![hung as Arc<dyn AgentToolHandler>],
        ))
        .await;
    let model = ScriptedEventModel::new([
        calls_events(&[("call-hung", "hung_permission", json!({}))]),
        deterministic_assistant_events("done"),
    ]);
    let service = AgentService::with_limits(Arc::new(model), tools.registries(), limits());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "go".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "done");
    assert!(Hung::cancelled(&seen));
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    let output = history
        .iter()
        .find_map(|item| match item {
            ConversationItem::ToolOutput {
                call_id, output, ..
            } if call_id == "call-hung" => Some(format!("{output:?}")),
            _ => None,
        })
        .expect("the call has an output");
    assert!(
        output.contains("permission check did not finish within"),
        "{output}"
    );
}
