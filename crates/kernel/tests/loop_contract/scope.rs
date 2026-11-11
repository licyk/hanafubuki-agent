//! The session's capability scope (site/content/docs/plugin-rules.en.mdx,
//! "Capability scope"): a tool it leaves out is absent exactly like an
//! unknown one, the kernel's own tools and `tool_search` survive any scope,
//! a plugin it turns off neither prompts nor continues the turn, approval
//! rules and tool lifecycle hooks still apply, the cap tightens the turn's
//! mode, children are never wider, the role changes only between turns,
//! and a contributor is told the tools the step offers.

use std::collections::BTreeSet;

use hanafubuki_agent_contracts::{
    PostToolUseDecision, PostToolUseFuture, PreToolUseDecision, PreToolUseFuture,
    PromptPrepareFuture, PromptStep, ToolLifecycleCall, ToolLifecycleHook,
    ToolLifecycleHookRegistration, ToolLifecycleHookRegistry, ToolLifecyclePhase,
};
use hanafubuki_agent_kernel::{
    AgentCapabilityScope, AgentChildSessionBase, AgentChildSessionRequest, AgentPermissionMode,
    AgentSessionOrigin, AgentSessionPersistence, AgentToolScope, ToolPattern,
};

use super::*;

fn names(texts: &[&str]) -> BTreeSet<ToolPattern> {
    texts
        .iter()
        .map(|text| ToolPattern::parse(text).unwrap())
        .collect()
}

fn scoped(scope: AgentCapabilityScope) -> AgentSessionConfig {
    let mut config = config();
    config.scope = scope;
    config
}

fn deny(texts: &[&str]) -> AgentCapabilityScope {
    AgentCapabilityScope {
        role: Some("narrow".to_string()),
        tools: AgentToolScope {
            allow: Vec::new(),
            deny: names(texts),
        },
        ..AgentCapabilityScope::default()
    }
}

fn plugins_off(plugins: &[&str]) -> AgentCapabilityScope {
    AgentCapabilityScope {
        role: Some("quiet".to_string()),
        plugins_off: plugins.iter().map(|plugin| plugin.to_string()).collect(),
        ..AgentCapabilityScope::default()
    }
}

fn call_events(
    call_id: &str,
    name: &str,
    arguments: serde_json::Value,
) -> Vec<ModelResult<ModelEvent>> {
    calls_events(&[(call_id, name, arguments)])
}

fn tool_names(request: &ModelRequest) -> Vec<String> {
    request
        .tools
        .iter()
        .filter_map(|tool| tool.name().map(|name| name.flattened_alias()))
        .collect()
}

/// The error the model read back for `call_id` in `request`.
fn output_of(request: &ModelRequest, call_id: &str) -> serde_json::Value {
    request
        .input
        .iter()
        .find_map(|item| match item {
            ModelInputItem::ToolOutput {
                call_id: id,
                output,
            } if id == call_id => output.structured_content.clone(),
            _ => None,
        })
        .expect("the call has its output")
}

fn has_section(request: &ModelRequest, kind: &str) -> bool {
    request.input.iter().any(
        |item| matches!(item, ModelInputItem::ContextFragment { kind: found, .. } if found == kind),
    )
}

#[tokio::test]
async fn a_tool_the_scope_leaves_out_is_absent_exactly_like_an_unknown_one() {
    // A direct and a deferred tool the scope denies, and one no plugin has.
    for name in ["control_instance", "configure_instance", "unknown_mutation"] {
        let model = ScriptedEventModel::new([
            call_events("scoped-call", name, json!({ "instanceId": "instance-a" })),
            deterministic_assistant_events("done"),
        ]);
        let tools = FakeTools::new([]).await;
        let service = AgentService::new(Arc::new(model.clone()), tools.registries());
        let session = service
            .create_session(scoped(deny(&["control_instance", "configure_*"])))
            .unwrap();

        let outcome = service
            .start_turn(session.id, "Call it".to_string())
            .await
            .unwrap();

        assert_eq!(outcome.final_answer, "done");
        assert!(tools.calls().is_empty());
        assert_eq!(tools.approved_calls(), 0);
        let requests = model.requests();
        assert_eq!(
            output_of(&requests[1], "scoped-call"),
            json!({ "error": format!("no tool named `{name}` is available in this step") }),
        );
        let offered = tool_names(&requests[0]);
        assert!(
            !offered.contains(&"control_instance".to_string()),
            "{offered:?}"
        );
        assert!(
            offered.contains(&"list_instances".to_string()),
            "{offered:?}"
        );
        // Left out on purpose: no omission is reported, and `tool_search`
        // has no deferred tool left to find.
        assert!(!has_section(&requests[0], "tool_catalog_omissions"));
        assert!(!offered.contains(&"tool_search".to_string()), "{offered:?}");
    }
}

#[tokio::test]
async fn the_kernels_tools_and_tool_search_survive_any_scope() {
    let model = ScriptedEventModel::new([deterministic_assistant_events("done")]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let scope = AgentCapabilityScope {
        role: Some("nothing".to_string()),
        tools: AgentToolScope {
            // Only the deferred fixture, and patterns naming the kernel's.
            allow: vec![names(&["configure_instance"])],
            deny: names(&[
                "tool_search",
                "update_plan",
                "new_context",
                "request_user_input",
            ]),
        },
        ..AgentCapabilityScope::default()
    };
    let session = service.create_session(scoped(scope)).unwrap();

    service
        .start_turn(session.id, "Hello".to_string())
        .await
        .unwrap();

    let mut offered = tool_names(&model.requests()[0]);
    offered.sort();
    assert_eq!(
        offered,
        [
            "new_context",
            "request_user_input",
            "tool_search",
            "update_plan"
        ]
    );
}

#[tokio::test]
async fn a_deferred_selection_of_a_tool_the_scope_leaves_out_is_ignored() {
    let model = ScriptedEventModel::new([
        call_events(
            "search",
            "tool_search",
            json!({ "query": "configure instance" }),
        ),
        deterministic_assistant_events("found"),
        call_events(
            "configure",
            "configure_instance",
            json!({ "instanceId": "instance-a" }),
        ),
        deterministic_assistant_events("not configured"),
    ]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(session.id, "Find the tool".to_string())
        .await
        .unwrap();
    // The search selected it: the step after offers it directly.
    assert!(tool_names(&model.requests()[1]).contains(&"configure_instance".to_string()));

    let switched = service
        .set_session_role(session.id, deny(&["configure_instance"]), None)
        .unwrap();
    assert_eq!(switched.role.as_deref(), Some("narrow"));
    assert!(switched.revision > session.revision);
    service
        .start_turn(session.id, "Configure it".to_string())
        .await
        .unwrap();

    let requests = model.requests();
    assert!(!tool_names(&requests[2]).contains(&"configure_instance".to_string()));
    assert_eq!(
        output_of(&requests[3], "configure"),
        json!({ "error": "no tool named `configure_instance` is available in this step" }),
    );
    assert!(tools.calls().is_empty());
    // A fork keeps the source's scope.
    let snapshot = service.snapshot(session.id).unwrap();
    let fork = service
        .fork_session_at_head(AgentSessionHeadForkRequest {
            source_session_id: session.id,
            source_generation: snapshot.generation,
            source_revision: snapshot.revision,
        })
        .unwrap();
    assert_eq!(fork.session.role.as_deref(), Some("narrow"));
    assert_eq!(fork.session.scope, switched.scope);
}

/// Denies every call before it runs.
struct DenyingHook;

impl ToolLifecycleHook for DenyingHook {
    fn applies(&self, phase: ToolLifecyclePhase, _call: &ToolLifecycleCall) -> bool {
        phase == ToolLifecyclePhase::PreToolUse
    }

    fn before_tool_use<'a>(
        &'a self,
        _call: ToolLifecycleCall,
        _cancellation: CancellationToken,
    ) -> PreToolUseFuture<'a> {
        Box::pin(async {
            Ok(PreToolUseDecision::Deny {
                reason: "the guard denies it".to_string(),
            })
        })
    }

    fn after_tool_use<'a>(
        &'a self,
        _call: ToolLifecycleCall,
        _output: hanafubuki_agent_tool_contracts::ToolOutput,
        _cancellation: CancellationToken,
    ) -> PostToolUseFuture<'a> {
        Box::pin(async { Ok(PostToolUseDecision::Pass) })
    }
}

/// Requires a person for every action that needs approval.
struct PersonRule;

impl ApprovalRule for PersonRule {
    fn id(&self) -> &str {
        "person"
    }

    fn evaluate(
        &self,
        _origin: Option<hanafubuki_agent_contracts::AgentSessionOrigin>,
        _action: &ApprovalAction,
    ) -> Option<ApprovalRuleVerdict> {
        Some(ApprovalRuleVerdict::RequireUser)
    }
}

#[tokio::test]
async fn tool_lifecycle_hooks_and_approval_rules_apply_whatever_the_scope_turns_off() {
    let model = ScriptedEventModel::new([
        call_events("read", "list_instances", json!({})),
        deterministic_assistant_events("denied"),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        json!({ "instances": [] }),
    )])
    .await;
    let hook = ToolLifecycleHookRegistration {
        provider_id: AgentContinuationHookProviderId::parse("guard".to_string()).unwrap(),
        hook_id: AgentContinuationHookId::parse("deny".to_string()).unwrap(),
        generation: AgentContinuationHookGeneration([0; 32]),
        priority: 0,
        hook: Arc::new(DenyingHook),
    };
    tools
        .mount(&registering::<ToolLifecycleHookRegistry>(
            "guard",
            vec![hook],
        ))
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service
        .create_session(scoped(plugins_off(&["guard"])))
        .unwrap();
    service
        .start_turn(session.id, "List them".to_string())
        .await
        .unwrap();
    assert!(
        tools.calls().is_empty(),
        "the hook of a plugin turned off still denies"
    );
    let denied = format!("{:?}", output_of(&model.requests()[1], "read"));
    assert!(denied.contains("the guard denies it"), "{denied}");

    // An approval rule of a plugin turned off still routes to a person,
    // even under full access.
    let model = ScriptedEventModel::new([
        mutation_tool_call_events(),
        deterministic_assistant_events("started"),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "control_instance",
        json!({ "instanceId": "instance-a", "action": "start" }),
        json!({ "operationId": "launch-1", "instanceId": "instance-a" }),
    )])
    .await;
    tools
        .mount(&registering::<ApprovalRuleRegistry>(
            "person-rule",
            vec![Arc::new(PersonRule) as Arc<dyn ApprovalRule>],
        ))
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service
        .create_session(scoped(plugins_off(&["person-rule"])))
        .unwrap();
    let session_id = session.id;
    let executing = service.clone();
    let turn = tokio::spawn(async move {
        executing
            .start_turn(
                session_id,
                permission_turn_input(AgentPermissionMode::FullAccess),
            )
            .await
    });
    let (active, request) = approval_request(&service, session_id, 0).await;
    service
        .resolve_approval(
            session_id,
            active.id,
            active.generation,
            request.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    assert_eq!(turn.await.unwrap().unwrap().final_answer, "started");
    assert_eq!(tools.approved_calls(), 1);
}

/// Adds one fragment to every request.
struct NoteContributor;

impl PromptContributor for NoteContributor {
    fn id(&self) -> &str {
        "note"
    }

    fn rank(&self) -> u32 {
        0
    }

    fn contribute(&self, _request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        vec![PromptFragment {
            kind: ContextSectionKind::parse("scope_note").unwrap(),
            version: None,
            content: "a note".to_string(),
        }]
    }
}

#[tokio::test]
async fn a_plugin_the_scope_turns_off_neither_prompts_nor_continues_the_turn() {
    let model = ScriptedEventModel::new([
        deterministic_assistant_events("first"),
        deterministic_assistant_events("second"),
        deterministic_assistant_events("third"),
    ]);
    let tools = FakeTools::new([]).await;
    let kind = ContextSectionKind::parse("scope_note").unwrap();
    tools
        .mount(&registering::<ContextRendererRegistry>(
            "noting",
            vec![plain_renderer(kind, ContextSlot::Capabilities)],
        ))
        .await;
    tools
        .mount(&registering::<PromptContributorRegistry>(
            "noting",
            vec![Arc::new(NoteContributor) as Arc<dyn PromptContributor>],
        ))
        .await;
    let hook = Arc::new(ContinueOnceHook::default());
    tools
        .mount(&registering::<ContinuationHookRegistry>(
            "noting",
            vec![AgentContinuationHookRegistration {
                provider_id: AgentContinuationHookProviderId::parse("noting".to_string()).unwrap(),
                hook_id: AgentContinuationHookId::parse("once".to_string()).unwrap(),
                generation: AgentContinuationHookGeneration([0; 32]),
                priority: 0,
                failure_policy: AgentContinuationHookFailurePolicy::Ignore,
                hook: hook.clone(),
            }],
        ))
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service
        .create_session(scoped(plugins_off(&["noting"])))
        .unwrap();

    let outcome = service
        .start_turn(session.id, "Hello".to_string())
        .await
        .unwrap();

    // The hook would have continued the turn once.
    assert_eq!(outcome.final_answer, "first");
    assert_eq!(model.requests().len(), 1);
    assert!(!has_section(&model.requests()[0], "scope_note"));
    // Without the scope, the same plugins do both.
    let session = service.create_session(config()).unwrap();
    let outcome = service
        .start_turn(session.id, "Hello".to_string())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "third");
    assert_eq!(model.requests().len(), 3);
    assert!(has_section(&model.requests()[1], "scope_note"));
}

/// Records the tools each step offers, as its contributors are told.
#[derive(Default)]
struct StepTools(Mutex<Vec<BTreeSet<String>>>);

impl PromptContributor for StepTools {
    fn id(&self) -> &str {
        "step-tools"
    }

    fn rank(&self) -> u32 {
        0
    }

    fn prepare<'a>(
        &'a self,
        step: &'a PromptStep,
        _cancellation: CancellationToken,
    ) -> PromptPrepareFuture<'a> {
        self.0.lock().unwrap().push(step.tools.as_ref().clone());
        Box::pin(async { Ok(TurnExtensions::default()) })
    }

    fn contribute(&self, _request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        Vec::new()
    }
}

#[tokio::test]
async fn a_contributor_is_told_the_tools_the_step_offers() {
    let model = ScriptedEventModel::new([
        deterministic_assistant_events("first"),
        deterministic_assistant_events("second"),
        deterministic_assistant_events("third"),
    ]);
    let tools = FakeTools::new([]).await;
    let recorder = Arc::new(StepTools::default());
    tools
        .mount(&registering::<PromptContributorRegistry>(
            "recording",
            vec![recorder.clone() as Arc<dyn PromptContributor>],
        ))
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let mut plan = config();
    plan.mode = hanafubuki_agent_kernel::AgentSessionMode::Plan;
    for config in [config(), scoped(deny(&["control_instance"])), plan] {
        let session = service.create_session(config).unwrap();
        service
            .start_turn(session.id, "Hello".to_string())
            .await
            .unwrap();
    }

    let steps = recorder.0.lock().unwrap().clone();
    let [unscoped, scoped, plan] = steps.as_slice() else {
        panic!("one step per turn: {steps:?}");
    };
    // Deferred tools are offered too, through the search.
    for name in ["control_instance", "configure_instance", "list_instances"] {
        assert!(unscoped.contains(name), "{unscoped:?}");
    }
    let requests = model.requests();
    assert!(tool_names(&requests[0])
        .iter()
        .all(|name| unscoped.contains(name)));
    // What the scope or the mode leaves out is not.
    assert!(!scoped.contains("control_instance"), "{scoped:?}");
    assert!(scoped.contains("configure_instance") && scoped.contains("list_instances"));
    assert!(!plan.contains("control_instance") && !plan.contains("configure_instance"));
    assert!(plan.contains("list_instances"), "{plan:?}");
}

#[tokio::test]
async fn the_cap_tightens_the_mode_the_turn_freezes() {
    let model = ScriptedEventModel::new([deterministic_assistant_events("done")]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(scoped(AgentCapabilityScope {
            role: Some("careful".to_string()),
            permission_mode_cap: Some(AgentPermissionMode::Manual),
            ..AgentCapabilityScope::default()
        }))
        .unwrap();
    service
        .start_turn(
            session.id,
            permission_turn_input(AgentPermissionMode::FullAccess),
        )
        .await
        .unwrap();
    let turn = service.snapshot(session.id).unwrap().turn.unwrap();
    assert_eq!(turn.permission_mode, AgentPermissionMode::Manual);
}

#[tokio::test]
async fn the_role_changes_only_between_turns() {
    let service = AgentService::new(
        Arc::new(CancellationOnlyModel),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let accepted = service
        .submit_turn_with_sink(session.id, "wait".to_string(), Arc::new(NoopEventSink))
        .unwrap();
    wait_for_active_turn(&service, session.id).await;

    assert_eq!(
        service.set_session_role(session.id, deny(&["list_*"]), None),
        Err(AgentError::TurnAlreadyRunning)
    );
    assert!(service.snapshot(session.id).unwrap().role.is_none());
    service
        .cancel_turn_confirmed(session.id, accepted.turn_id, accepted.generation)
        .await
        .unwrap();
    let switched = service
        .set_session_role(
            session.id,
            deny(&["list_*"]),
            Some("Only list.".to_string()),
        )
        .unwrap();
    assert_eq!(switched.role.as_deref(), Some("narrow"));
    // Back to `default`: the empty scope.
    let cleared = service
        .set_session_role(session.id, AgentCapabilityScope::default(), None)
        .unwrap();
    assert_eq!((cleared.role, cleared.scope), (None, None));
}

#[tokio::test]
async fn a_child_is_never_wider_than_its_parent() {
    let model = ScriptedEventModel::new([deterministic_assistant_events("done")]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let parent = AgentCapabilityScope {
        role: Some("parent".to_string()),
        tools: AgentToolScope {
            allow: vec![names(&["list_*", "read_task_log"])],
            deny: names(&["list_tasks"]),
        },
        plugins_off: ["titles".to_string()].into(),
        permission_mode_cap: Some(AgentPermissionMode::AutoReview),
    };
    let session = service.create_session(scoped(parent.clone())).unwrap();
    let request = |scope| AgentChildSessionRequest {
        parent_session_id: session.id,
        origin: AgentSessionOrigin::Subagent,
        persistence: AgentSessionPersistence::Durable,
        base: AgentChildSessionBase::Empty,
        context: Vec::new(),
        scope,
        role_instructions: None,
    };

    // A child asking for nothing narrower keeps its parent's scope whole.
    let child = service
        .create_child_session(request(AgentCapabilityScope::default()))
        .unwrap();
    assert_eq!(child.role.as_deref(), Some("parent"));
    assert_eq!(child.scope, parent.summary());

    // One that tries to widen only narrows further.
    let wider = AgentCapabilityScope {
        role: Some("child".to_string()),
        tools: AgentToolScope {
            allow: vec![names(&["*"])],
            deny: names(&["read_task_log"]),
        },
        plugins_off: ["goal".to_string()].into(),
        permission_mode_cap: Some(AgentPermissionMode::FullAccess),
    };
    let child = service.create_child_session(request(wider)).unwrap();
    let summary = child.scope.unwrap();
    assert_eq!(child.role.as_deref(), Some("child"));
    assert_eq!((summary.allow_lists, summary.deny_patterns), (2, 2));
    assert_eq!(summary.plugins_off, ["goal", "titles"]);
    assert_eq!(
        summary.permission_mode_cap,
        Some(AgentPermissionMode::AutoReview)
    );
}
