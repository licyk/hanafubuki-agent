//! What a guest contributes, through the registries the kernel reads: its
//! declared tools (approved, then run as its `tool` call), context sections,
//! prompt fragment, hooks and observer, each an effect of its fiber; what
//! cannot be honoured is left out with an issue. A guest that fails or
//! hangs costs only its own contributions, which retire with it and come
//! back with its restart.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_approval::{
    ApprovalActionKind, ApprovalContext, ApprovalDecisionSource, ApprovalService,
    PermissionRequirement,
};
use hanafubuki_agent_contracts::{
    AgentContinuationContext, AgentContinuationHookDecision, AgentGeneration, AgentNotification,
    AgentRevision, AgentSessionId, AgentSessionMode, AgentToolError, AgentTurnId, AgentTurnOrigin,
    CancellationToken, ContextRegistry, ContextRendererRegistry, ContextRequest,
    ContinuationHookRegistry, NoopAgentToolProgressSink, ObserverRegistry, PostToolUseDecision,
    PreToolUseDecision, PromptContributorRegistry, PromptRequest, PromptStep, RegistryService,
    SourcedTool, ToolCallContext, ToolInvocation, ToolLifecycleCall, ToolLifecycleHookRegistry,
    ToolSourceRegistry, ToolSourceStep, TurnExtensions,
};
use hanafubuki_agent_host::RestartPolicy;
use hanafubuki_agent_packages::{ContributedSection, ContributedTool, PackageContributes};
use hanafubuki_agent_plugin_packages::{GuestPackage, GuestState};
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use serde_json::{json, Map, Value};

use crate::fixture::{eventually, running, Fixture, Options};

/// A package whose guest is in `mode` and declares one of everything, and
/// some declarations no host could honour.
fn contributing(fixture: &Fixture, mode: &str) -> GuestPackage {
    let mut package = fixture.package("demo", mode);
    let tool = |name: &str| ContributedTool {
        name: name.to_string(),
        description: format!("The {name} tool."),
        input_schema: Map::from_iter([("type".to_string(), json!("object"))]),
        timeout: None,
    };
    let section = |kind: &str| ContributedSection {
        kind: kind.to_string(),
        renderer: None,
    };
    package.contributes = PackageContributes {
        tools: ["echo", "text", "fail", "exit", "bad name!"]
            .into_iter()
            .map(tool)
            .collect(),
        context_sections: ["demo_board", "empty_board", "broken_board", "environment"]
            .into_iter()
            .map(section)
            .collect(),
        prompt_slot: Some("capabilities".to_string()),
        tool_lifecycle_hooks: true,
        continuation_hooks: true,
        observers: vec!["turn_finished".to_string(), "turn_completed".to_string()],
    };
    package
}

pub fn entries<S: RegistryService>(fixture: &Fixture) -> Vec<S::Item>
where
    S::Item: Clone,
{
    let registry = fixture.registry::<S>();
    let snapshot = registry.registry().snapshot();
    snapshot.items().cloned().collect()
}

pub fn owners<S: RegistryService>(fixture: &Fixture) -> Vec<String> {
    let registry = fixture.registry::<S>();
    let snapshot = registry.registry().snapshot();
    snapshot
        .entries()
        .iter()
        .map(|entry| entry.owner().to_string())
        .collect()
}

fn step() -> ToolSourceStep {
    ToolSourceStep {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(2),
        generation: AgentGeneration::new(1),
        session_revision: AgentRevision::new(1),
        memory_contribution: false,
        cancellation: CancellationToken::default(),
    }
}

fn invocation(arguments: Value) -> ToolInvocation {
    ToolInvocation {
        context: ToolCallContext {
            session_id: AgentSessionId::new(1),
            turn_id: AgentTurnId::new(2),
            generation: AgentGeneration::new(1),
            mode: AgentSessionMode::Default,
            session_revision: AgentRevision::new(1),
            call_id: "call-1".to_string(),
            image_input: false,
            origin: Default::default(),
        },
        arguments,
        cancellation: CancellationToken::default(),
        progress: Arc::new(NoopAgentToolProgressSink),
        plan_leases: Vec::new(),
        turn: TurnExtensions::default(),
    }
}

pub fn context_request() -> ContextRequest {
    ContextRequest {
        session_id: AgentSessionId::new(1),
        mode: AgentSessionMode::Default,
        permission_mode: Default::default(),
        user_message: Some("hello".to_string()),
        turn: TurnExtensions::default(),
    }
}

fn lifecycle_call(tool_name: &str) -> ToolLifecycleCall {
    ToolLifecycleCall {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(2),
        generation: AgentGeneration::new(1),
        call_id: "call-1".to_string(),
        tool_name: tool_name.to_string(),
        arguments: json!({}),
    }
}

fn continuation(answer: &str) -> AgentContinuationContext {
    AgentContinuationContext {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(2),
        generation: AgentGeneration::new(1),
        revision: AgentRevision::new(1),
        sampling_step: 1,
        candidate_final_answer: answer.to_string(),
        background: Default::default(),
    }
}

/// The tools the guest's source plans for a step.
pub async fn planned(fixture: &Fixture) -> Result<Vec<SourcedTool>, AgentToolError> {
    let sources = entries::<ToolSourceRegistry>(fixture);
    assert_eq!(sources.len(), 1);
    let step = step();
    crate::watchdog::step("planning the tools");
    let contribution = sources[0].contribute(&step).await?;
    crate::watchdog::step("planned the tools");
    Ok(contribution.tools)
}

/// Runs `tool` with `arguments`, approved as its permission asks.
pub async fn run(tool: &SourcedTool, arguments: Value) -> Result<ToolOutput, AgentToolError> {
    crate::watchdog::step(format!(
        "running the tool {}",
        tool.handler.descriptor().name.flattened_alias()
    ));
    let call = invocation(arguments.clone());
    let PermissionRequirement::ReviewRequired(action) = tool.handler.permission(&call).await?
    else {
        panic!("a plugin's tool asks for approval");
    };
    assert_eq!(action.kind, ApprovalActionKind::PluginToolCall);
    let context = ApprovalContext::from_canonical_bytes("call-1".to_string(), b"scope").unwrap();
    let approval = ApprovalService::new(Arc::new(
        hanafubuki_agent_approval::test_support::ManualApprovalClock::new(1),
    ))
    .authorize(
        action,
        &context,
        Duration::from_secs(60),
        ApprovalDecisionSource::FullAccess,
    )
    .unwrap();
    let output = tool.handler.invoke(call, Some(approval)).await;
    crate::watchdog::step("the tool ran");
    Ok(output?.output)
}

pub fn tool<'a>(tools: &'a [SourcedTool], name: &str) -> &'a SourcedTool {
    tools
        .iter()
        .find(|tool| tool.handler.descriptor().name.flattened_alias() == name)
        .unwrap_or_else(|| panic!("no tool {name} in {tools:?}"))
}

pub async fn a_guest_contributes_what_its_manifest_declares() {
    let fixture = Fixture::start(Options::default()).await;
    let package = contributing(&fixture, "run");
    let id = package.id.key();
    fixture.apply(vec![package.clone()]).await;
    let status = fixture
        .wait(&id, "its contributions", |status| {
            running(status) && !status.issues.is_empty()
        })
        .await;
    let issues = status.issues.join("\n");
    assert!(issues.contains("`bad name!`"), "{issues}");
    assert!(issues.contains("`environment`"), "{issues}");
    assert!(issues.contains("`turn_completed`"), "{issues}");
    assert_eq!(status.issues.len(), 3, "{issues}");
    let guest = format!("guest:{id}");

    // Tools: planned under the plugin's name space, approved, then run.
    assert_eq!(owners::<ToolSourceRegistry>(&fixture), vec![guest.clone()]);
    let tools = planned(&fixture).await.unwrap();
    let mut names = tools
        .iter()
        .map(|tool| tool.handler.descriptor().name.flattened_alias())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        ["demo__echo", "demo__exit", "demo__fail", "demo__text"]
    );
    let echo = tool(&tools, "demo__echo");
    assert_eq!(echo.provider_id.as_str(), "plugin.demo.local");
    assert_eq!(
        planned(&fixture).await.unwrap()[0].generation,
        tools[0].generation,
        "a tool keeps its identity while its declaration does not change"
    );
    let output = run(echo, json!({ "q": "x" })).await.unwrap();
    assert_eq!(
        output.structured_content,
        Some(json!({
            "echoed": { "q": "x" },
            "call": { "sessionId": 1, "turnId": 2, "callId": "call-1" },
        }))
    );
    let output = run(tool(&tools, "demo__text"), json!({})).await.unwrap();
    assert_eq!(
        output.content,
        vec![ToolOutputContent::Text {
            text: "plain text".to_string()
        }]
    );
    let Err(AgentToolError::Failed(failure)) = run(tool(&tools, "demo__fail"), json!({})).await
    else {
        panic!("a failing tool is the call's error");
    };
    assert!(failure.contains("SECRET-TOOL-FAILURE"), "{failure}");
    let unapproved = echo.handler.invoke(invocation(json!({})), None).await;
    assert!(matches!(unapproved, Err(AgentToolError::Failed(_))));

    // Context sections: one contributor and one renderer per kind.
    assert_eq!(
        owners::<ContextRegistry>(&fixture),
        vec![guest.clone(), guest.clone(), guest.clone()]
    );
    let contributors = entries::<ContextRegistry>(&fixture);
    let contributor = |kind: &str| {
        contributors
            .iter()
            .find(|contributor| contributor.kinds()[0].as_str() == kind)
            .unwrap()
            .clone()
    };
    let request = context_request();
    let sections = contributor("demo_board")
        .contribute(&request, CancellationToken::default())
        .await
        .unwrap();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].content, "demo_board for session 1");
    let empty = contributor("empty_board")
        .contribute(&request, CancellationToken::default())
        .await
        .unwrap();
    assert!(empty.is_empty());
    let broken = contributor("broken_board")
        .contribute(&request, CancellationToken::default())
        .await
        .unwrap_err();
    // What the kernel is told, and logs, is the Agent's words, never the
    // guest's.
    assert!(!broken.contains("SECRET"), "{broken}");
    let renderers = fixture.registry::<ContextRendererRegistry>().snapshot();
    for kind in ["demo_board", "empty_board", "broken_board", "plugin_demo"] {
        let kind = hanafubuki_agent_contracts::ContextSectionKind::parse(kind).unwrap();
        assert!(renderers.find(&kind).is_some(), "no renderer of {kind}");
    }

    // The prompt: prepared by the guest host's one contributor.
    assert_eq!(
        owners::<PromptContributorRegistry>(&fixture),
        vec!["plugin-guests".to_string()]
    );
    let contributor = entries::<PromptContributorRegistry>(&fixture).remove(0);
    let step = PromptStep {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(2),
        generation: AgentGeneration::new(1),
        sampling_step: 3,
        tools: Arc::default(),
    };
    let prepared = contributor
        .prepare(&step, CancellationToken::default())
        .await
        .unwrap();
    let fragments = contributor.contribute(&PromptRequest {
        turn: &TurnExtensions::default(),
        prepared: Some(&prepared),
        context_window_tokens: None,
    });
    assert_eq!(fragments.len(), 1);
    assert_eq!(fragments[0].kind.as_str(), "plugin_demo");
    assert_eq!(fragments[0].content, "Prompt for step 3.");

    // The hooks: after the user's and the goal's.
    let hooks = entries::<ToolLifecycleHookRegistry>(&fixture);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].provider_id.as_str(), "plugin.demo.local");
    assert_eq!(hooks[0].priority, 200);
    let hook = &hooks[0].hook;
    let cancellation = CancellationToken::default();
    assert_eq!(
        hook.before_tool_use(lifecycle_call("forbidden"), cancellation.clone())
            .await,
        Ok(PreToolUseDecision::Deny {
            reason: "the test guest denies it".to_string()
        })
    );
    assert_eq!(
        hook.before_tool_use(lifecycle_call("demo__echo"), cancellation.clone())
            .await,
        Ok(PreToolUseDecision::Allow)
    );
    assert_eq!(
        hook.after_tool_use(
            lifecycle_call("demo__echo"),
            ToolOutput::structured(json!({}), false),
            cancellation.clone()
        )
        .await,
        Ok(PostToolUseDecision::AddContext {
            fragments: vec!["after demo__echo".to_string()]
        })
    );
    let continuations = entries::<ContinuationHookRegistry>(&fixture);
    assert_eq!(continuations.len(), 1);
    let hook = &continuations[0].hook;
    assert_eq!(
        hook.evaluate(continuation("more please"), cancellation.clone())
            .await,
        Ok(AgentContinuationHookDecision::Continue {
            fragments: vec!["keep going".to_string()]
        })
    );
    assert_eq!(
        hook.evaluate(continuation("stop now"), cancellation.clone())
            .await,
        Ok(AgentContinuationHookDecision::Stop)
    );
    assert_eq!(
        hook.evaluate(continuation("done"), cancellation.clone())
            .await,
        Ok(AgentContinuationHookDecision::Pass)
    );

    // The observer: only the declared events reach the guest.
    let observers = entries::<ObserverRegistry>(&fixture);
    assert_eq!(observers.len(), 1);
    observers[0].observe(&AgentNotification::TurnAdmitted {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(2),
        generation: AgentGeneration::new(1),
        origin: AgentTurnOrigin::User,
        automatic_title: None,
    });
    observers[0].observe(&AgentNotification::TurnFinished {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(2),
        generation: AgentGeneration::new(1),
    });
    let expected = json!([{ "event": "turn_finished", "sessionId": 1, "turnId": 2 }]);
    let deadline = Instant::now() + crate::fixture::DEADLINE;
    loop {
        let observed = fixture.call(&package, "observed", Value::Null).await;
        if observed.as_ref() == Ok(&expected) {
            break;
        }
        assert!(Instant::now() < deadline, "{observed:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub async fn a_guests_contributions_retire_with_it_and_its_failures_stay_its_own() {
    let fixture = Fixture::start(Options {
        call_timeout: Duration::from_secs(2),
        policy: RestartPolicy {
            delays: vec![Duration::from_secs(2); 3],
            window: Duration::from_secs(600),
        },
        ..Options::default()
    })
    .await;
    let package = contributing(&fixture, "hang-contributions");
    let id = package.id.key();
    fixture.apply(vec![package.clone()]).await;
    fixture
        .wait(&id, "its contributions", |status| {
            running(status) && !status.issues.is_empty()
        })
        .await;

    // A context section the guest does not give in time is left out, and
    // the guest is killed for it.
    let contributor = entries::<ContextRegistry>(&fixture).remove(0);
    let started = Instant::now();
    let left_out = contributor
        .contribute(&context_request(), CancellationToken::default())
        .await;
    assert!(left_out.is_err());
    assert!(started.elapsed() < Duration::from_secs(10));
    fixture
        .wait(&id, "the restart", |status| {
            status.state == GuestState::Restarting
        })
        .await;
    // Its contributions retired with it, and the guest host's prompt
    // contributor asks no guest that does not run.
    eventually("its contributions retired", || {
        owners::<ToolSourceRegistry>(&fixture).is_empty()
            && owners::<ContextRegistry>(&fixture).is_empty()
            && owners::<ObserverRegistry>(&fixture).is_empty()
    })
    .await;
    let prompts = entries::<PromptContributorRegistry>(&fixture).remove(0);
    let step = PromptStep {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(2),
        generation: AgentGeneration::new(1),
        sampling_step: 1,
        tools: Arc::default(),
    };
    let prepared = prompts
        .prepare(&step, CancellationToken::default())
        .await
        .unwrap();
    assert!(prompts
        .contribute(&PromptRequest {
            turn: &TurnExtensions::default(),
            prepared: Some(&prepared),
            context_window_tokens: None,
        })
        .is_empty());

    // They come back with its restart; a prompt it does not give in time
    // costs only that prompt.
    let back = fixture
        .wait(&id, "its restart", |status| {
            running(status) && status.restarts == 1 && !status.issues.is_empty()
        })
        .await;
    eventually("its contributions back", || {
        owners::<ToolSourceRegistry>(&fixture).len() == 1
    })
    .await;
    let started = Instant::now();
    let prepared = prompts
        .prepare(&step, CancellationToken::default())
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(prompts
        .contribute(&PromptRequest {
            turn: &TurnExtensions::default(),
            prepared: Some(&prepared),
            context_window_tokens: None,
        })
        .is_empty());
    fixture
        .wait(&id, "the second restart", |status| {
            running(status) && status.restarts == 2 && status.process_id != back.process_id
        })
        .await;
    eventually("its contributions back again", || {
        owners::<ToolSourceRegistry>(&fixture).len() == 1
    })
    .await;

    // A tool whose guest exits is the call's error; the guest restarts.
    let tools = planned(&fixture).await.unwrap();
    let exited = run(tool(&tools, "demo__exit"), json!({})).await;
    assert!(
        matches!(exited, Err(AgentToolError::Failed(_))),
        "{exited:?}"
    );
    fixture
        .wait(&id, "the third restart", |status| {
            running(status) && status.restarts == 3
        })
        .await;
}
