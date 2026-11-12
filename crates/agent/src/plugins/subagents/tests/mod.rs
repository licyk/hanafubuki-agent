//! The `subagents` plugin on a running Agent over a scripted model: an
//! agent spawned by the parent's model runs its task and `wait_agent`
//! returns its answer; messages steer a running agent or wait for an idle
//! one; an interrupt; the depth and running limits; the inherited
//! permission mode; and the plugin turned off. Agents spawned as roles
//! are `roles.rs`'s.

mod roles;

use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_contracts::{
    AgentToolError, AgentToolHandler, CancellationToken, ToolCallContext, ToolInvocation,
    TurnExtensions,
};
use hanafubuki_agent_kernel::{
    AgentError, AgentPermissionMode, AgentSessionId, AgentSessionMode, AgentSessionOrigin,
};
use serde_json::{json, Value};

use super::service::{AgentRunStatus, Delivery, ListedAgent, SpawnRequest, WaitOutcome};
use super::tools::{SubagentTool, SubagentToolKind};
use super::{AgentSubagentState, AgentSubagentStatus, SubagentService, SUBAGENTS_PLUGIN};
use crate::test_support::{
    assistant_events, configure_test_provider, service_with_host, test_host, test_tool_context,
    tool_calls_events, FakeServer, TestPaths, TEST_SHUTDOWN_DEADLINE,
};
use crate::{AgentApplicationService, AgentPluginState};

/// A started Agent on `server`.
async fn started(name: &str, server: &FakeServer) -> (AgentApplicationService, TestPaths) {
    let (service, paths, _) = service_with_host(name, test_host, |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    service.start_plugins().await;
    (service, paths)
}

async fn subagents(service: &AgentApplicationService) -> Arc<SubagentService> {
    service
        .feature(
            SUBAGENTS_PLUGIN,
            |subagents: Arc<SubagentService>| async move { Ok(subagents) },
        )
        .await
        .unwrap()
}

fn finish(service: AgentApplicationService, paths: TestPaths) {
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn turn_running(service: &AgentApplicationService, session_id: AgentSessionId) -> bool {
    service
        .snapshot(session_id)
        .unwrap()
        .turn
        .is_some_and(|turn| !turn.phase.is_terminal())
}

fn body(server: &FakeServer, index: usize) -> String {
    String::from_utf8(server.requests()[index].body.clone()).unwrap()
}

fn spawn(task_name: &str, message: &str) -> SpawnRequest {
    SpawnRequest {
        task_name: task_name.to_string(),
        message: message.to_string(),
        fork: false,
        role: Default::default(),
    }
}

const NO_WAIT: Duration = Duration::from_secs(20);

/// Runs `kind` for `session` as the kernel does once its permission is
/// settled.
async fn call(
    subagents: &Arc<SubagentService>,
    kind: SubagentToolKind,
    session_id: AgentSessionId,
    arguments: Value,
) -> Result<Value, AgentToolError> {
    let context = test_tool_context();
    let result = SubagentTool::new(kind, subagents.clone())
        .unwrap()
        .invoke(
            ToolInvocation {
                context: ToolCallContext {
                    session_id,
                    turn_id: context.turn_id,
                    generation: context.generation,
                    mode: AgentSessionMode::Default,
                    session_revision: context.session_revision,
                    call_id: context.call_id,
                    image_input: false,
                },
                arguments,
                cancellation: CancellationToken::default(),
                progress: context.progress,
                plan_leases: Vec::new(),
                turn: TurnExtensions::default(),
            },
            None,
        )
        .await?;
    Ok(result.output.structured_content.unwrap())
}

/// The parent's model spawns an agent and waits for it: the agent runs its
/// task on its own conversation with the parent's model and permission
/// mode, and the wait hands the parent its answer.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_spawned_agent_runs_its_task_and_the_wait_returns_its_answer() {
    let server = FakeServer::start(vec![
        (
            tool_calls_events(&[
                (
                    "call-spawn",
                    "spawn_agent",
                    r#"{"task_name":"counter","message":"Count the files.","fork_turns":"none"}"#,
                ),
                ("call-wait", "wait_agent", r#"{"timeout_ms":20000}"#),
            ]),
            Duration::ZERO,
        ),
        (assistant_events("Twelve files."), Duration::ZERO),
        (assistant_events("There are twelve."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = started("subagents-spawn", &server).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .set_session_permission_mode(parent.id, AgentPermissionMode::FullAccess)
        .unwrap();

    let outcome = service
        .start_turn(parent.id, "Delegate the count.".to_string().into())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "There are twelve.");
    let parent_request = body(&server, 0);
    assert!(parent_request.contains(r#""name":"spawn_agent""#));
    assert!(parent_request.contains("<subagents>"));
    let child_request = body(&server, 1);
    assert!(child_request.contains("Count the files."));
    assert!(
        !child_request.contains("Delegate the count."),
        "fork_turns none inherits nothing"
    );
    assert!(
        !child_request.contains("<subagents>"),
        "an agent is not told to spawn"
    );
    let waited = body(&server, 2);
    assert!(waited.contains("Wait completed."), "{waited}");
    assert!(waited.contains("Twelve files."), "{waited}");

    let agents = service.subagents(parent.id).await.unwrap();
    let child = agents[0].session_id;
    assert_eq!(
        agents,
        vec![AgentSubagentStatus {
            session_id: child,
            title: "counter".to_string(),
            status: AgentSubagentState::Idle,
            last_answer: Some("Twelve files.".to_string()),
        }]
    );
    let snapshot = service.snapshot(child).unwrap();
    assert_eq!(snapshot.parent_session_id, Some(parent.id));
    assert_eq!(snapshot.origin, Some(AgentSessionOrigin::Subagent));
    assert!(!snapshot.ephemeral);
    assert_eq!(
        snapshot.turn.unwrap().permission_mode,
        AgentPermissionMode::FullAccess,
        "an agent runs with its parent's permission mode"
    );
    finish(service, paths);
}

/// By default an agent starts from its parent's conversation as it stood
/// before the turn that spawns it: the earlier turns, not the request that
/// asked for it.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_spawned_agent_inherits_the_conversation_before_the_spawning_turn() {
    let server = FakeServer::start(vec![
        (assistant_events("Planned."), Duration::ZERO),
        (
            tool_calls_events(&[
                (
                    "call-spawn",
                    "spawn_agent",
                    r#"{"task_name":"helper","message":"Do step two."}"#,
                ),
                ("call-wait", "wait_agent", "{}"),
            ]),
            Duration::ZERO,
        ),
        (assistant_events("Step two done."), Duration::ZERO),
        (assistant_events("All done."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = started("subagents-fork", &server).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .start_turn(parent, "Plan the two steps.".to_string().into())
        .await
        .unwrap();

    let outcome = service
        .start_turn(parent, "Now delegate step two.".to_string().into())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "All done.");
    let child_request = body(&server, 2);
    assert!(child_request.contains("Plan the two steps."));
    assert!(child_request.contains("Planned."));
    assert!(child_request.contains("Do step two."));
    assert!(
        !child_request.contains("Now delegate step two."),
        "the spawning turn is not inherited"
    );
    assert!(body(&server, 3).contains("Step two done."));
    let child = service.subagents(parent).await.unwrap()[0].session_id;
    assert_eq!(
        service.snapshot(child).unwrap().forked_from_session_id,
        Some(parent)
    );
    finish(service, paths);
}

/// A message steers a running agent, waits for an idle one's next turn,
/// which a follow-up task starts; an interrupt stops a turn without
/// reporting it to the parent's wait.
#[tokio::test(flavor = "multi_thread")]
async fn messages_steer_a_running_agent_and_wait_for_an_idle_ones_next_turn() {
    let server = FakeServer::start(vec![
        (assistant_events("Started."), Duration::from_secs(1)),
        (assistant_events("Checked the tests."), Duration::ZERO),
        (assistant_events("Summarized."), Duration::ZERO),
        (assistant_events("Too late."), Duration::from_secs(30)),
    ])
    .await;
    let (service, paths) = started("subagents-messages", &server).await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let cancellation = CancellationToken::default();

    let child = subagents
        .spawn(parent, spawn("worker", "Start."))
        .await
        .unwrap();
    wait_until("the agent runs", || turn_running(&service, child)).await;
    assert_eq!(
        subagents
            .send_message(parent, "worker", "Also check the tests.".into())
            .await
            .unwrap(),
        Delivery::Steered
    );
    assert_eq!(
        subagents
            .wait(parent, NO_WAIT, &cancellation)
            .await
            .unwrap(),
        WaitOutcome::Updates(vec![ListedAgent {
            name: "worker".to_string(),
            id: child,
            role: "default".to_string(),
            status: AgentRunStatus::Completed(Some("Checked the tests.".to_string())),
        }])
    );
    assert!(body(&server, 1).contains("Also check the tests."));

    assert_eq!(
        subagents
            .send_message(parent, &child.to_string(), "Keep it short.".into())
            .await
            .unwrap(),
        Delivery::Queued
    );
    assert_eq!(
        subagents
            .followup_task(parent, "worker", "Now summarize.".into())
            .await
            .unwrap(),
        Delivery::Started
    );
    assert!(matches!(
        subagents.wait(parent, NO_WAIT, &cancellation).await.unwrap(),
        WaitOutcome::Updates(updates)
            if updates[0].status == AgentRunStatus::Completed(Some("Summarized.".to_string()))
    ));
    let followup = body(&server, 2);
    assert!(followup.contains("Keep it short.") && followup.contains("Now summarize."));

    subagents
        .followup_task(parent, "worker", "Take long.".into())
        .await
        .unwrap();
    wait_until("the follow-up samples", || server.requests().len() == 4).await;
    assert_eq!(
        subagents.interrupt(parent, "worker").await.unwrap(),
        AgentRunStatus::Running
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while subagents.agents(parent).await.unwrap()[0].status != AgentRunStatus::Interrupted {
        assert!(Instant::now() < deadline, "the interrupt is recorded");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        subagents
            .wait(parent, NO_WAIT, &cancellation)
            .await
            .unwrap(),
        WaitOutcome::Idle,
        "an interrupted turn is no update, and nothing runs"
    );
    assert_eq!(
        service.subagents(parent).await.unwrap()[0].status,
        AgentSubagentState::Idle
    );
    finish(service, paths);
}

/// Only a session without a parent spawns, at most three agents of a
/// parent run at once, and a task name is taken once; each refusal is the
/// tool's error, and nothing is created for it.
#[tokio::test(flavor = "multi_thread")]
async fn the_depth_and_running_limits_refuse_a_spawn_with_a_tool_error() {
    let server = FakeServer::start(vec![
        (assistant_events("Late."), Duration::from_secs(30));
        3
    ])
    .await;
    let (service, paths) = started("subagents-limits", &server).await;
    let subagents = subagents(&service).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    let spawn_args = |name: &str| json!({ "task_name": name, "message": "Wait here." });

    for name in ["one", "two", "three"] {
        let spawned = call(
            &subagents,
            SubagentToolKind::SpawnAgent,
            parent,
            spawn_args(name),
        )
        .await
        .unwrap();
        assert_eq!(spawned["task_name"], name);
    }
    assert_eq!(
        call(
            &subagents,
            SubagentToolKind::SpawnAgent,
            parent,
            spawn_args("four")
        )
        .await,
        Err(AgentToolError::Failed(
            "at most 3 agents can run at once; wait for one to finish or interrupt one".into()
        ))
    );
    let child = subagents.agents(parent).await.unwrap()[0].id;
    assert_eq!(
        call(
            &subagents,
            SubagentToolKind::SpawnAgent,
            child,
            spawn_args("nested")
        )
        .await,
        Err(AgentToolError::Failed(
            "a sub-agent cannot spawn agents of its own; only the main conversation spawns agents"
                .into()
        ))
    );
    assert_eq!(
        call(&subagents, SubagentToolKind::SpawnAgent, parent, spawn_args("one")).await,
        Err(AgentToolError::Failed(
            "an agent named `one` already exists; send it a follow-up task or choose another task_name"
                .into()
        ))
    );
    let listed = call(&subagents, SubagentToolKind::ListAgents, parent, json!({}))
        .await
        .unwrap();
    assert_eq!(
        listed["agents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|agent| (agent["agent_name"].clone(), agent["agent_status"].clone()))
            .collect::<Vec<_>>(),
        vec![
            (json!("one"), json!("running")),
            (json!("two"), json!("running")),
            (json!("three"), json!("running")),
        ]
    );
    assert!(subagents.agents(child).await.unwrap().is_empty());

    for name in ["one", "two", "three"] {
        let interrupted = call(
            &subagents,
            SubagentToolKind::InterruptAgent,
            parent,
            json!({ "target": name }),
        )
        .await
        .unwrap();
        assert_eq!(interrupted["previous_status"], "running");
    }
    finish(service, paths);
}

/// With the plugin off its tools and guidance are gone from the requests
/// and its calls are unavailable; the rest of the Agent works on.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_subagents_plugin_off_the_tools_are_gone_and_the_calls_unavailable() {
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO)]).await;
    let (service, paths) = started("subagents-off", &server).await;
    service
        .set_plugin_enabled(SUBAGENTS_PLUGIN.into(), false)
        .await
        .unwrap();
    wait_until("the subagents plugin is off", || {
        service.plugin_status().iter().any(|status| {
            status.name == SUBAGENTS_PLUGIN && status.state == AgentPluginState::Disabled
        })
    })
    .await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    let unavailable = AgentError::FeatureUnavailable {
        feature: "subagents".to_string(),
        reason: "the subagents Agent plugin is unavailable: it is turned off".to_string(),
    };
    assert_eq!(service.subagents(parent).await, Err(unavailable.clone()));
    assert_eq!(service.interrupt_subagent(parent).await, Err(unavailable));
    service
        .start_turn(parent, "Hello.".to_string().into())
        .await
        .unwrap();
    let request = body(&server, 0);
    assert!(!request.contains("spawn_agent") && !request.contains("wait_agent"));
    assert!(!request.contains("<subagents>"));
    finish(service, paths);
}

/// A write of the running-agent limit is taken in place: the service, and
/// the runs it tracks, stay, and the guidance says the new limit. A write
/// of a wait bound, which `wait_agent`'s spec states, restarts the plugin
/// with the new spec. A value out of bounds is refused before it is
/// written.
#[tokio::test(flavor = "multi_thread")]
async fn an_option_write_is_taken_in_place_or_restarts_the_plugin() {
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO)]).await;
    let (service, paths) = started("subagents-options", &server).await;
    let options = |value: Value| value.as_object().unwrap().clone();
    let before = subagents(&service).await;
    assert_eq!(before.limits().maximum_running_agents, 3);
    let refused = service
        .set_plugin_options(
            SUBAGENTS_PLUGIN.into(),
            options(json!({ "maximumRunningAgents": 17 })),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("between 1 and 16"), "{refused}");

    service
        .set_plugin_options(
            SUBAGENTS_PLUGIN.into(),
            options(json!({ "maximumRunningAgents": 5 })),
        )
        .await
        .unwrap();
    wait_until("the limit is taken in place", || {
        before.limits().maximum_running_agents == 5
    })
    .await;
    assert!(Arc::ptr_eq(&before, &subagents(&service).await));
    let status = service
        .plugin_status()
        .into_iter()
        .find(|status| status.name == SUBAGENTS_PLUGIN)
        .unwrap();
    let shown = status.options.unwrap();
    assert_eq!(
        shown.values[0].value,
        Some(hanafubuki_agent_config::SettingValue::Integer(5))
    );
    assert!(shown.values[0].layer.is_some() && shown.values[1].layer.is_none());
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .start_turn(parent, "Hello.".to_string().into())
        .await
        .unwrap();
    assert!(body(&server, 0).contains("At most 5 sub-agents run at once"));

    service
        .set_plugin_options(
            SUBAGENTS_PLUGIN.into(),
            options(json!({ "defaultWaitSeconds": 60 })),
        )
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let after = loop {
        let current = service
            .feature(
                SUBAGENTS_PLUGIN,
                |subagents: Arc<SubagentService>| async move { Ok(subagents) },
            )
            .await;
        match current {
            Ok(current) if !Arc::ptr_eq(&before, &current) => break current,
            _ => {
                assert!(Instant::now() < deadline, "the plugin did not restart");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    };
    let limits = after.limits();
    assert_eq!(limits.maximum_running_agents, 5);
    assert_eq!(limits.waits.default, Duration::from_secs(60));
    let wait = SubagentTool::new(SubagentToolKind::WaitAgent, after).unwrap();
    assert!(
        wait.descriptor()
            .input_schema
            .to_string()
            .contains("Defaults to 60000"),
        "{}",
        wait.descriptor().input_schema
    );
    finish(service, paths);
}
