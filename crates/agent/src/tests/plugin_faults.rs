//! A plugin whose code panics is stopped and shown `Failed` with the panic,
//! while the turn takes that extension point's own error path and the other
//! plugins keep working; a retry starts it again.
//!
//! The `faulty` test plugin registers one entry at every extension point and
//! panics in the one a test arms. The `healthy` plugin runs beside it with a
//! tool and an observer, next to the application's real plugins. A plugin
//! whose tool name is taken fails the same way, at start.

use crate::test_support::TestPaths;
use crate::AgentConfig;
use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, AuthorizedAction,
    PermissionRequirement,
};
use hanafubuki_agent_contracts::{
    spawn_scoped_worker, AgentActivityAction, AgentContinuationContext, AgentContinuationHook,
    AgentContinuationHookDecision, AgentContinuationHookFailurePolicy, AgentContinuationHookFuture,
    AgentContinuationHookGeneration, AgentContinuationHookId, AgentContinuationHookProviderId,
    AgentContinuationHookRegistration, AgentMemoryContextEffect, AgentNotification, AgentObserver,
    AgentToolError, AgentToolHandler, AgentToolSource, ApprovalRule, ApprovalRuleRegistry,
    ApprovalRuleVerdict, ContextContributor, ContextFuture, ContextRegistry, ContextRequest,
    ContextSectionKind, ContinuationHookRegistry, ObserverRegistry, PluginFaults,
    PromptContributor, PromptContributorRegistry, PromptFragment, PromptPrepareFuture,
    PromptRequest, PromptStep, Register, ToolAvailability, ToolDescriptor, ToolEffect, ToolFuture,
    ToolInvocation, ToolPlanContext, ToolRegistry, ToolResult, ToolSourceContribution,
    ToolSourceRegistry, ToolSourceStep, ToolSourceTrust, TurnAdmissionContributor,
    TurnAdmissionFuture, TurnAdmissionRegistry, TurnAdmissionRequest, TurnExtensions,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_tool_contracts::{ToolExecutionClass, ToolExposure, ToolName};
use kardia_core::{Context, Inject, PluginDef, Service};
use serde_json::Value;

use super::host::{AgentPluginGroup, AgentPluginRow, AgentPluginState, AgentPluginStatus};
use super::*;

/// The extension point the `faulty` plugin panics in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    Availability,
    Permission,
    Context,
    Invoke,
    Source,
    PromptPrepare,
    PromptContribute,
    Admission,
    /// Not a panic: the turn admission contributor returns an error.
    AdmissionError,
    Observer,
    Approval,
    Hook,
    Worker,
}

#[derive(Default)]
struct Plan {
    armed: Mutex<Option<Point>>,
    invoked: AtomicUsize,
}

impl Plan {
    fn arm(&self, point: Point) {
        *self.armed.lock().unwrap() = Some(point);
    }

    fn disarm(&self) {
        *self.armed.lock().unwrap() = None;
    }

    fn armed(&self, point: Point) -> bool {
        *self.armed.lock().unwrap() == Some(point)
    }

    /// Panics when `point` is armed.
    fn trip(&self, point: Point) {
        if self.armed(point) {
            panic!("faulty plugin panicked in {point:?}");
        }
    }
}

fn descriptor(name: &'static str) -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::parse(name).unwrap(),
        description: format!("The {name} test tool."),
        input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        exposure: ToolExposure::Direct,
        execution_class: ToolExecutionClass::ConcurrentRead,
        effect: ToolEffect::ReadOnly,
        action: AgentActivityAction::fixed(name),
        memory_effect: AgentMemoryContextEffect::None,
        timeout: None,
    }
}

/// A tool of the `faulty` plugin (panicking where armed) or the `healthy`
/// one (`plan` is `None`).
struct Tool {
    descriptor: ToolDescriptor,
    plan: Option<Arc<Plan>>,
}

impl AgentToolHandler for Tool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn availability(&self, _plan: &ToolPlanContext) -> ToolAvailability {
        if let Some(plan) = &self.plan {
            plan.trip(Point::Availability);
        }
        ToolAvailability::Function
    }

    fn permission<'a>(
        &'a self,
        _invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async move {
            let Some(plan) = &self.plan else {
                return Ok(PermissionRequirement::NotRequired);
            };
            plan.trip(Point::Permission);
            if !plan.armed(Point::Approval) {
                return Ok(PermissionRequirement::NotRequired);
            }
            let action = ApprovalAction::from_canonical_bytes(
                ApprovalActionKind::DomainMutation,
                ApprovalRiskClass::High,
                ApprovalOrigin::Model,
                "run the faulty tool".to_string(),
                b"faulty",
            )
            .map_err(|error| AgentToolError::Failed(error.to_string()))?;
            Ok(PermissionRequirement::ReviewRequired(action))
        })
    }

    fn invoke(
        &self,
        _invocation: ToolInvocation,
        _approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            if let Some(plan) = &self.plan {
                plan.trip(Point::Invoke);
                plan.invoked.fetch_add(1, Ordering::SeqCst);
            }
            Ok(ToolResult::success(
                json!({ "ran": self.descriptor.name.qualified() }),
            ))
        })
    }
}

/// Every other extension point of the `faulty` plugin.
struct Faulty(Arc<Plan>);

impl AgentToolSource for Faulty {
    fn id(&self) -> &str {
        "faulty-source"
    }

    fn trust(&self) -> ToolSourceTrust {
        ToolSourceTrust::Optional
    }

    fn contribute<'a>(
        &'a self,
        _step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>> {
        Box::pin(async move {
            self.0.trip(Point::Source);
            Ok(ToolSourceContribution::default())
        })
    }
}

static FAULTY_KINDS: [ContextSectionKind; 1] = [ContextSectionKind::fixed("faulty_section")];

impl ContextContributor for Faulty {
    fn id(&self) -> &str {
        "faulty-context"
    }

    fn kinds(&self) -> &[ContextSectionKind] {
        &FAULTY_KINDS
    }

    fn contribute<'a>(
        &'a self,
        _request: &'a ContextRequest,
        _cancellation: CancellationToken,
    ) -> ContextFuture<'a> {
        Box::pin(async move {
            self.0.trip(Point::Context);
            Ok(Vec::new())
        })
    }
}

impl PromptContributor for Faulty {
    fn id(&self) -> &str {
        "faulty-prompt"
    }

    fn rank(&self) -> u32 {
        9
    }

    fn prepare<'a>(
        &'a self,
        _step: &'a PromptStep,
        _cancellation: CancellationToken,
    ) -> PromptPrepareFuture<'a> {
        Box::pin(async move {
            self.0.trip(Point::PromptPrepare);
            Ok(TurnExtensions::default())
        })
    }

    fn contribute(&self, _request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        self.0.trip(Point::PromptContribute);
        Vec::new()
    }
}

impl TurnAdmissionContributor for Faulty {
    fn id(&self) -> &str {
        "faulty-admission"
    }

    fn rank(&self) -> u32 {
        9
    }

    fn contribute<'a>(
        &'a self,
        _request: &'a TurnAdmissionRequest,
        _cancellation: CancellationToken,
    ) -> TurnAdmissionFuture<'a> {
        Box::pin(async move {
            self.0.trip(Point::Admission);
            if self.0.armed(Point::AdmissionError) {
                return Err("catalog offline".to_string());
            }
            Ok(TurnExtensions::default())
        })
    }
}

impl AgentObserver for Faulty {
    fn id(&self) -> &str {
        "faulty-observer"
    }

    fn observe(&self, _notification: &AgentNotification) {
        self.0.trip(Point::Observer);
    }
}

impl ApprovalRule for Faulty {
    fn id(&self) -> &str {
        "faulty-rule"
    }

    fn evaluate(
        &self,
        _origin: Option<hanafubuki_agent_contracts::AgentSessionOrigin>,
        _action: &ApprovalAction,
    ) -> Option<ApprovalRuleVerdict> {
        self.0.trip(Point::Approval);
        None
    }
}

impl AgentContinuationHook for Faulty {
    fn evaluate<'a>(
        &'a self,
        _context: AgentContinuationContext,
        _cancellation: CancellationToken,
    ) -> AgentContinuationHookFuture<'a> {
        Box::pin(async move {
            self.0.trip(Point::Hook);
            Ok(AgentContinuationHookDecision::Pass)
        })
    }
}

fn faulty_plugin(plan: Arc<Plan>) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let plan = plan.clone();
        async move {
            let faulty = Arc::new(Faulty(plan.clone()));
            ctx.get::<ToolRegistry>()?.register(Arc::new(Tool {
                descriptor: descriptor("faulty_tool"),
                plan: Some(plan.clone()),
            }) as Arc<dyn AgentToolHandler>)?;
            ctx.get::<ToolSourceRegistry>()?
                .register(faulty.clone() as Arc<dyn AgentToolSource>)?;
            ctx.get::<PromptContributorRegistry>()?
                .register(faulty.clone() as Arc<dyn PromptContributor>)?;
            ctx.get::<ContextRegistry>()?
                .register(faulty.clone() as Arc<dyn ContextContributor>)?;
            ctx.get::<TurnAdmissionRegistry>()?
                .register(faulty.clone() as Arc<dyn TurnAdmissionContributor>)?;
            ctx.get::<ObserverRegistry>()?
                .register(faulty.clone() as Arc<dyn AgentObserver>)?;
            ctx.get::<ApprovalRuleRegistry>()?
                .register(faulty.clone() as Arc<dyn ApprovalRule>)?;
            ctx.get::<ContinuationHookRegistry>()?
                .register(AgentContinuationHookRegistration {
                    provider_id: AgentContinuationHookProviderId::parse("faulty".into()).unwrap(),
                    hook_id: AgentContinuationHookId::parse("hook".into()).unwrap(),
                    generation: AgentContinuationHookGeneration([1; 32]),
                    priority: 0,
                    failure_policy: AgentContinuationHookFailurePolicy::Ignore,
                    hook: faulty,
                })?;
            spawn_scoped_worker(&ctx, "faulty-worker", move |cancellation| async move {
                while !cancellation.is_cancelled() {
                    plan.trip(Point::Worker);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })?;
            Ok(())
        }
    })
    .name("faulty")
    .inject(
        Inject::new()
            .with(ToolRegistry::NAME)
            .with(ToolSourceRegistry::NAME)
            .with(PromptContributorRegistry::NAME)
            .with(ContextRegistry::NAME)
            .with(TurnAdmissionRegistry::NAME)
            .with(ObserverRegistry::NAME)
            .with(ApprovalRuleRegistry::NAME)
            .with(ContinuationHookRegistry::NAME)
            .with(PluginFaults::NAME),
    )
}

/// Counts the turns it was told finished.
#[derive(Default)]
struct HealthyObserver(AtomicUsize);

impl AgentObserver for HealthyObserver {
    fn id(&self) -> &str {
        "healthy-observer"
    }

    fn observe(&self, notification: &AgentNotification) {
        if matches!(notification, AgentNotification::TurnFinished { .. }) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn healthy_plugin(observer: Arc<HealthyObserver>) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let observer = observer.clone();
        async move {
            ctx.get::<ToolRegistry>()?.register(Arc::new(Tool {
                descriptor: descriptor("healthy_tool"),
                plan: None,
            }) as Arc<dyn AgentToolHandler>)?;
            ctx.get::<ObserverRegistry>()?
                .register(observer as Arc<dyn AgentObserver>)?;
            Ok(())
        }
    })
    .name("healthy")
    .inject(
        Inject::new()
            .with(ToolRegistry::NAME)
            .with(ObserverRegistry::NAME),
    )
}

/// A test plugin's row, with its descriptor.
fn described(
    name: &'static str,
    plugin: PluginDef,
) -> (AgentPluginRow, super::host::AgentPluginDescriptor) {
    (
        AgentPluginRow::unconfigured(AgentPluginGroup::Capability, plugin),
        super::host::AgentPluginDescriptor::new(name, "a test plugin"),
    )
}

/// A service over `server` with the application's plugins, `faulty` and
/// `healthy`, a session, and the plugins started.
struct Fixture {
    service: AgentApplicationService,
    paths: TestPaths,
    plan: Arc<Plan>,
    healthy: Arc<HealthyObserver>,
    session_id: hanafubuki_agent_kernel::AgentSessionId,
}

impl Fixture {
    async fn new(name: &str, server: &FakeServer) -> Self {
        let plan = Arc::new(Plan::default());
        let healthy = Arc::new(HealthyObserver::default());
        let paths = test_paths(name);
        let config = AgentConfig::new(&paths.config_dir);
        configure_test_provider(&config, &server.base_url, None);
        let service = AgentApplicationService::new_with_test_plugins(
            &paths,
            config,
            AgentStoragePaths::under(paths.data_dir.join("agent-test")),
            SkillRoots {
                system: paths.data_dir.join("agent-test/skills-bundled"),
                user: paths.data_dir.join("agent-test/skills-user"),
                install_bundled_system_skills: false,
                state_file: paths.data_dir.join("agent-test/skills-state.json"),
            },
            vec![
                described("faulty", faulty_plugin(plan.clone())),
                described("healthy", healthy_plugin(healthy.clone())),
            ],
        )
        .unwrap();
        let session_id = service
            .create_session(AgentSessionMode::Default, None)
            .await
            .unwrap()
            .id;
        let fixture = Self {
            service,
            paths,
            plan,
            healthy,
            session_id,
        };
        assert_eq!(fixture.status("faulty").state, AgentPluginState::Active);
        fixture
    }

    fn status(&self, name: &str) -> AgentPluginStatus {
        self.service
            .plugin_status()
            .into_iter()
            .find(|status| status.name == name)
            .unwrap_or_else(|| panic!("no plugin {name}"))
    }

    async fn turn(
        &self,
        text: &str,
    ) -> Result<hanafubuki_agent_kernel::AgentTurnOutcome, hanafubuki_agent_kernel::AgentError>
    {
        self.service
            .start_turn(self.session_id, text.to_string().into())
            .await
    }

    /// Waits until the host stopped `faulty` for its panic in `point`, checks
    /// that every other plugin kept running, then retries it.
    async fn assert_failed_then_retry(self, point: Point, described: &str) {
        let deadline = Instant::now() + TEST_WAIT;
        while self.status("faulty").state != AgentPluginState::Failed {
            assert!(Instant::now() < deadline, "faulty never failed: {point:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let error = self.status("faulty").error.unwrap();
        assert_eq!(
            error,
            format!("panicked in {described}: faulty plugin panicked in {point:?}")
        );
        assert!(self.status("faulty").contributions.tools.is_empty());
        for status in self.service.plugin_status() {
            if status.name != "faulty" && status.enabled {
                assert_eq!(status.state, AgentPluginState::Active, "{}", status.name);
            }
        }
        assert_eq!(self.status("healthy").contributions.tools, ["healthy_tool"]);

        self.plan.disarm();
        self.service.retry_plugin("faulty").await.unwrap();
        let retried = self.status("faulty");
        assert_eq!(retried.state, AgentPluginState::Active);
        assert_eq!(retried.error, None);
        assert_eq!(retried.contributions.tools, ["faulty_tool"]);
        assert_eq!(
            self.service
                .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
            []
        );
        let _ = std::fs::remove_dir_all(&self.paths.app_data_dir);
    }
}

/// The text of the tool output the model was sent in `request`.
fn tool_output(server: &FakeServer, request: usize) -> String {
    let body: Value = serde_json::from_slice(&server.requests()[request].body).unwrap();
    body["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .map(|item| item.to_string())
        .collect()
}

fn offered_tools(server: &FakeServer, request: usize) -> Vec<String> {
    let body: Value = serde_json::from_slice(&server.requests()[request].body).unwrap();
    body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn answer(text: &str) -> (Vec<u8>, Duration) {
    (assistant_events(text), Duration::ZERO)
}

fn call(name: &str) -> (Vec<u8>, Duration) {
    (tool_events(name, "{}"), Duration::ZERO)
}

#[tokio::test]
async fn a_panicking_tool_invoke_is_a_tool_error_and_the_turn_goes_on() {
    let server = FakeServer::start(vec![
        call("faulty_tool"),
        call("healthy_tool"),
        answer("done"),
    ])
    .await;
    let fixture = Fixture::new("fault-invoke", &server).await;
    fixture.plan.arm(Point::Invoke);

    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "done");

    assert!(tool_output(&server, 1).contains("the tool failed with an internal error"));
    assert!(!tool_output(&server, 1).contains("faulty plugin panicked"));
    assert!(tool_output(&server, 2).contains("healthy_tool"));
    fixture
        .assert_failed_then_retry(Point::Invoke, "tool invoke")
        .await;
}

#[tokio::test]
async fn a_panicking_tool_permission_is_a_tool_error_and_the_tool_never_runs() {
    let server = FakeServer::start(vec![call("faulty_tool"), answer("done")]).await;
    let fixture = Fixture::new("fault-permission", &server).await;
    fixture.plan.arm(Point::Permission);

    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "done");

    assert!(tool_output(&server, 1).contains("the tool failed with an internal error"));
    assert_eq!(fixture.plan.invoked.load(Ordering::SeqCst), 0);
    fixture
        .assert_failed_then_retry(Point::Permission, "tool permission")
        .await;
}

#[tokio::test]
async fn a_panicking_tool_availability_leaves_the_tool_out_of_the_step() {
    let server = FakeServer::start(vec![answer("done")]).await;
    let fixture = Fixture::new("fault-availability", &server).await;
    fixture.plan.arm(Point::Availability);

    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "done");

    let offered = offered_tools(&server, 0);
    assert!(!offered.contains(&"faulty_tool".to_string()), "{offered:?}");
    assert!(offered.contains(&"healthy_tool".to_string()), "{offered:?}");
    fixture
        .assert_failed_then_retry(Point::Availability, "tool availability")
        .await;
}

#[tokio::test]
async fn a_panicking_optional_tool_source_is_omitted_from_the_step() {
    let server = FakeServer::start(vec![answer("done")]).await;
    let fixture = Fixture::new("fault-source", &server).await;
    fixture.plan.arm(Point::Source);

    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "done");

    fixture
        .assert_failed_then_retry(Point::Source, "tool source")
        .await;
}

#[tokio::test]
async fn a_panicking_prompt_prepare_leaves_its_values_out() {
    let server = FakeServer::start(vec![answer("done")]).await;
    let fixture = Fixture::new("fault-prompt-prepare", &server).await;
    fixture.plan.arm(Point::PromptPrepare);

    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "done");

    fixture
        .assert_failed_then_retry(Point::PromptPrepare, "prompt prepare")
        .await;
}

#[tokio::test]
async fn a_panicking_prompt_contribute_contributes_no_fragment() {
    let server = FakeServer::start(vec![answer("done")]).await;
    let fixture = Fixture::new("fault-prompt-contribute", &server).await;
    fixture.plan.arm(Point::PromptContribute);

    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "done");

    fixture
        .assert_failed_then_retry(Point::PromptContribute, "prompt contribute")
        .await;
}

#[tokio::test]
async fn a_panicking_turn_admission_contributor_is_left_out_of_that_admission() {
    let server = FakeServer::start(vec![answer("admitted"), answer("admitted")]).await;
    let fixture = Fixture::new("fault-admission", &server).await;
    fixture.plan.arm(Point::Admission);

    // The turn is admitted without the contributor's values; the panic
    // still fails its plugin.
    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "admitted");
    // Once the host stopped the plugin, its contributor is gone and the
    // next turn is admitted.
    let deadline = Instant::now() + TEST_WAIT;
    while fixture.status("faulty").state != AgentPluginState::Failed {
        assert!(Instant::now() < deadline, "faulty never failed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        fixture.turn("again").await.unwrap().final_answer,
        "admitted"
    );

    fixture
        .assert_failed_then_retry(Point::Admission, "turn admission contributor")
        .await;
}

#[tokio::test]
async fn a_failing_turn_admission_contributor_degrades_its_plugin_until_it_works() {
    let server = FakeServer::start(vec![answer("admitted"), answer("admitted")]).await;
    let fixture = Fixture::new("fault-admission-error", &server).await;
    fixture.plan.arm(Point::AdmissionError);

    // An error is no fault: the turn runs without the contributor and its
    // plugin stays active, with the issue as its warning.
    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "admitted");
    let status = fixture.status("faulty");
    assert_eq!(status.state, AgentPluginState::Active);
    assert_eq!(
        status.warning.as_deref(),
        Some("turn admission contributor faulty-admission: catalog offline")
    );
    assert!(fixture
        .service
        .plugin_status()
        .iter()
        .all(|plugin| plugin.name == "faulty" || plugin.warning.is_none()));

    fixture.plan.disarm();
    assert_eq!(
        fixture.turn("again").await.unwrap().final_answer,
        "admitted"
    );
    assert_eq!(fixture.status("faulty").warning, None);
}

#[tokio::test]
async fn a_panicking_context_contributor_is_left_out_of_that_admission() {
    let server = FakeServer::start(vec![answer("admitted"), answer("admitted")]).await;
    let fixture = Fixture::new("fault-context", &server).await;
    fixture.plan.arm(Point::Context);

    // The turn is admitted without the contributor's sections; the panic
    // still fails its plugin.
    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "admitted");
    let deadline = Instant::now() + TEST_WAIT;
    while fixture.status("faulty").state != AgentPluginState::Failed {
        assert!(Instant::now() < deadline, "faulty never failed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        fixture.turn("again").await.unwrap().final_answer,
        "admitted"
    );

    fixture
        .assert_failed_then_retry(Point::Context, "context contributor")
        .await;
}

#[tokio::test]
async fn a_panicking_observer_leaves_the_turn_and_the_other_observers_alone() {
    let server = FakeServer::start(vec![answer("done")]).await;
    let fixture = Fixture::new("fault-observer", &server).await;
    fixture.plan.arm(Point::Observer);

    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "done");

    assert_eq!(fixture.healthy.0.load(Ordering::SeqCst), 1);
    fixture
        .assert_failed_then_retry(Point::Observer, "observer")
        .await;
}

#[tokio::test]
async fn a_panicking_approval_rule_fails_closed() {
    let server = FakeServer::start(vec![call("faulty_tool"), answer("done")]).await;
    let fixture = Fixture::new("fault-approval", &server).await;
    fixture.plan.arm(Point::Approval);

    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "done");

    assert!(tool_output(&server, 1)
        .contains("an approval rule of the faulty plugin failed with an internal error"));
    assert_eq!(fixture.plan.invoked.load(Ordering::SeqCst), 0);
    assert!(!fixture
        .service
        .events_since(fixture.session_id, None)
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. })));
    fixture
        .assert_failed_then_retry(Point::Approval, "approval rule")
        .await;
}

#[tokio::test]
async fn a_panicking_continuation_hook_fails_under_its_failure_policy() {
    let server = FakeServer::start(vec![answer("done")]).await;
    let fixture = Fixture::new("fault-hook", &server).await;
    fixture.plan.arm(Point::Hook);

    // The hook's policy is `Ignore`: its failure is skipped.
    assert_eq!(fixture.turn("go").await.unwrap().final_answer, "done");

    fixture
        .assert_failed_then_retry(Point::Hook, "continuation hook")
        .await;
}

#[tokio::test]
async fn a_panicking_scoped_worker_fails_its_plugin() {
    let server = FakeServer::start(Vec::new()).await;
    let fixture = Fixture::new("fault-worker", &server).await;
    fixture.plan.arm(Point::Worker);

    fixture
        .assert_failed_then_retry(Point::Worker, "worker faulty-worker")
        .await;
}

#[tokio::test]
async fn turning_a_faulted_plugin_off_and_on_starts_it_again() {
    let server = FakeServer::start(Vec::new()).await;
    let fixture = Fixture::new("fault-disable", &server).await;
    fixture.plan.arm(Point::Worker);
    let deadline = Instant::now() + TEST_WAIT;
    while fixture.status("faulty").state != AgentPluginState::Failed {
        assert!(Instant::now() < deadline, "faulty never failed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    fixture.plan.disarm();

    // Turning a faulted plugin off and on again starts it, like a retry.
    fixture
        .service
        .set_plugin_enabled("faulty".to_string(), false)
        .await
        .unwrap();
    assert_eq!(fixture.status("faulty").state, AgentPluginState::Disabled);
    fixture
        .service
        .set_plugin_enabled("faulty".to_string(), true)
        .await
        .unwrap();
    while fixture.status("faulty").state != AgentPluginState::Active {
        assert!(
            Instant::now() < deadline + Duration::from_secs(5),
            "faulty never restarted"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(fixture.status("faulty").error, None);
    assert_eq!(
        fixture
            .service
            .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(&fixture.paths.app_data_dir);
}

/// A plugin that registers `squatted` after a tool of its own.
fn squatting_plugin(name: &'static str, squatted: &'static str) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| async move {
        let tools = ctx.get::<ToolRegistry>()?;
        for tool in [format!("{name}_tool"), squatted.to_string()] {
            tools.register(Arc::new(Tool {
                descriptor: ToolDescriptor {
                    name: ToolName::parse(tool).unwrap(),
                    action: AgentActivityAction::USE_TOOL,
                    ..descriptor("healthy_tool")
                },
                plan: None,
            }) as Arc<dyn AgentToolHandler>)?;
        }
        Ok(())
    })
    .name(name)
    .inject(Inject::new().with(ToolRegistry::NAME))
}

/// A tool named like `tool_search`, a kernel tool or a registered tool is
/// refused at registration: the plugin that registered it fails at start
/// with nothing registered, and the other plugins and the turns go on.
#[tokio::test]
async fn a_plugin_registering_a_taken_tool_name_fails_alone() {
    let server = FakeServer::start(vec![call("healthy_tool"), answer("done")]).await;
    let paths = test_paths("taken-tool-names");
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, &server.base_url, None);
    let healthy = Arc::new(HealthyObserver::default());
    let service = AgentApplicationService::new_with_test_plugins(
        &paths,
        config,
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        vec![
            described("healthy", healthy_plugin(healthy.clone())),
            described("searcher", squatting_plugin("searcher", "tool_search")),
            described("planner", squatting_plugin("planner", "update_plan")),
            described("copycat", squatting_plugin("copycat", "healthy_tool")),
        ],
    )
    .unwrap();
    let session_id = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let status = |name: &str| {
        service
            .plugin_status()
            .into_iter()
            .find(|status| status.name == name)
            .unwrap()
    };
    for (name, error) in [
        ("searcher", "tool_search is reserved for the Agent kernel"),
        ("planner", "update_plan is reserved for the Agent kernel"),
        ("copycat", "agent-tools already contains healthy_tool"),
    ] {
        let status = status(name);
        assert_eq!(status.state, AgentPluginState::Failed, "{name}");
        assert!(status.error.unwrap().contains(error), "{name}");
        assert!(status.contributions.tools.is_empty(), "{name}");
    }
    for status in service.plugin_status() {
        if !["searcher", "planner", "copycat"].contains(&status.name.as_str()) && status.enabled {
            assert_eq!(status.state, AgentPluginState::Active, "{}", status.name);
        }
    }

    let outcome = service
        .start_turn(session_id, "go".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "done");
    let offered = offered_tools(&server, 0);
    assert_eq!(
        offered
            .iter()
            .filter(|name| ["healthy_tool", "update_plan"].contains(&name.as_str()))
            .count(),
        2
    );
    assert!(!offered
        .iter()
        .any(|name| ["searcher_tool", "planner_tool", "copycat_tool"].contains(&name.as_str())));
    assert!(tool_output(&server, 1).contains("healthy_tool"));
    assert_eq!(healthy.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(&paths.app_data_dir);
}
