//! The tool pipeline, driven through the real kernel and plugins by a
//! scripted model (`FakeServer`) and a scripted MCP server: the step's gate,
//! each output committed as the calls before it are, a cancel that commits
//! what finished, approval outside the gate and "allow for this session",
//! an MCP call past its timeout, and the unknown-tool and bad-argument
//! outputs. The kernel's own limits and descriptor timeout are pinned in
//! `hanafubuki-agent-kernel`'s loop contract (`turn_limits`, `handlers`).

use hanafubuki_agent_kernel::AgentEventKind;
use serde_json::Value;

use super::*;

/// The synthetic output of a call whose result was not committed.
const INTERRUPTED: &str = "{\"error\":\"tool execution ended before the result was durably committed\",\"replayed\":false,\"status\":\"interrupted\"}";

/// One sampling step: `skills_list` (concurrent read), the MCP `echo`
/// (Exclusive), then `skills_list` again. The read queued behind
/// the exclusive call waits for it, and each output is committed as soon as
/// every earlier call's is: the first read's while the exclusive call still
/// runs.
#[tokio::test]
async fn an_exclusive_call_serializes_the_step_and_each_output_commits_in_call_order() {
    let mcp = echo_server().await;
    let model = FakeServer::start(vec![
        (
            calls_events(&[
                ("call-1", None, "skills_list", "{}"),
                (
                    "call-2",
                    Some("mcp__fixture"),
                    "echo",
                    "{\"value\":\"held\"}",
                ),
                ("call-3", None, "skills_list", "{}"),
            ]),
            Duration::ZERO,
        ),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = pipeline_service("pipeline-gate", &model, &mcp).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    mcp.hold.arm();
    let turn = spawn_turn(&service, session.id);
    mcp.hold.entered().await;
    wait_for_event(&service, session.id, "completed call-1").await;

    // A grace for `call-3` to run if it could; it cannot while `call-2` holds
    // the step's exclusive access.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let events = tool_events_of(&service, session.id);
    assert_eq!(
        events,
        [
            "started call-1",
            "started call-2",
            "started call-3",
            "finished call-1",
            "completed call-1",
        ]
    );

    mcp.hold.release();
    assert_eq!(turn.await.unwrap().unwrap().final_answer, "Done.");
    let events = tool_events_of(&service, session.id);
    assert_eq!(
        events,
        [
            "started call-1",
            "started call-2",
            "started call-3",
            "finished call-1",
            "completed call-1",
            "finished call-2",
            "completed call-2",
            "finished call-3",
            "completed call-3",
        ]
    );
    let outputs = call_outputs(&model.requests()[1].body);
    assert_eq!(
        outputs
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["call-1", "call-2", "call-3"]
    );
    assert_eq!(
        outputs[1].1,
        json!([{"type": "input_text", "text": "echo called with {\"value\":\"held\"}"}])
    );
    finish(service, paths);
}

/// A cancel while one call has finished and another is still running: the
/// finished call's output is committed (its effects happened), and only the
/// running one gets the synthetic interrupted output.
#[tokio::test]
async fn a_cancel_mid_step_commits_the_finished_call_and_interrupts_the_running_one() {
    let mcp = echo_server().await;
    let model = FakeServer::start(vec![
        (
            calls_events(&[
                ("call-1", None, "skills_list", "{}"),
                (
                    "call-2",
                    Some("mcp__fixture"),
                    "echo",
                    "{\"value\":\"held\"}",
                ),
            ]),
            Duration::ZERO,
        ),
        (assistant_events("After the cancel."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = pipeline_service("pipeline-cancel", &model, &mcp).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    mcp.hold.arm();
    let turn = spawn_turn(&service, session.id);
    mcp.hold.entered().await;
    wait_for_event(&service, session.id, "completed call-1").await;

    let identity = service.snapshot(session.id).unwrap().turn.unwrap();
    service
        .cancel_turn(session.id, identity.id, identity.generation)
        .await
        .unwrap();
    assert!(turn.await.unwrap().is_err());
    assert!(!tool_events_of(&service, session.id).contains(&"completed call-2".to_string()));

    service
        .start_turn(session.id, "Go on.".into())
        .await
        .unwrap();
    // The model reads an interrupted output right after its call, so it
    // comes before the earlier call's committed one.
    let outputs = call_outputs(&model.requests()[1].body);
    assert_eq!(
        outputs[0],
        ("call-2".to_string(), Value::String(INTERRUPTED.to_string()))
    );
    assert_eq!(outputs[1].0, "call-1");
    assert!(outputs[1].1.to_string().contains("entries"), "{outputs:?}");
    finish(service, paths);
}

/// The MCP call waits for the user's approval without holding the step's
/// gate, so the read after it runs meanwhile (its output waits to commit
/// behind the call before it). "Allow for this session" then answers the
/// next step's call of the same tool without asking.
#[tokio::test]
async fn a_call_waiting_for_approval_lets_the_step_proceed_and_allow_for_session_holds() {
    let mcp = echo_server().await;
    let model = FakeServer::start(vec![
        (
            calls_events(&[
                (
                    "call-1",
                    Some("mcp__fixture"),
                    "echo",
                    "{\"value\":\"first\"}",
                ),
                ("call-2", None, "skills_list", "{}"),
            ]),
            Duration::ZERO,
        ),
        (
            calls_events(&[(
                "call-3",
                Some("mcp__fixture"),
                "echo",
                "{\"value\":\"second\"}",
            )]),
            Duration::ZERO,
        ),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let mut server = mcp.config("fixture", hanafubuki_agent_mcp::McpExposurePolicy::Direct);
    server.approval_policy = hanafubuki_agent_mcp::McpApprovalPolicy::Ask;
    let (service, paths) = pipeline_service_with("pipeline-approval", &model, server).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let turn = spawn_turn(&service, session.id);
    wait_for_event(&service, session.id, "finished call-2").await;
    let requests = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let requests = approval_requests(&service, session.id);
            if !requests.is_empty() {
                break requests;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("call-1 asks for approval");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].offers_allow_for_session);
    assert!(!tool_events_of(&service, session.id)
        .iter()
        .any(|event| event.starts_with("completed")));

    let identity = service.snapshot(session.id).unwrap().turn.unwrap();
    service
        .resolve_approval(
            session.id,
            identity.id,
            identity.generation,
            requests[0].request_id,
            hanafubuki_agent_kernel::ApprovalUserDecision::AllowForSession,
        )
        .unwrap();
    assert_eq!(turn.await.unwrap().unwrap().final_answer, "Done.");
    assert_eq!(approval_requests(&service, session.id).len(), 1);
    let outputs = call_outputs(&model.requests()[2].body);
    assert_eq!(
        outputs
            .iter()
            .map(|(id, output)| (id.as_str(), output.to_string().contains("error")))
            .collect::<Vec<_>>(),
        [("call-1", false), ("call-2", false), ("call-3", false)]
    );
    assert!(outputs[2].1.to_string().contains("second"), "{outputs:?}");
    finish(service, paths);
}

/// An MCP call that does not answer within its server's call timeout gets
/// a timeout error, and the turn continues.
#[tokio::test]
async fn an_mcp_call_past_its_timeout_gets_an_error_and_the_turn_continues() {
    let mcp = echo_server().await;
    let model = FakeServer::start(vec![
        (
            calls_events(&[(
                "call-1",
                Some("mcp__fixture"),
                "echo",
                "{\"value\":\"held\"}",
            )]),
            Duration::ZERO,
        ),
        (assistant_events("Recovered."), Duration::ZERO),
    ])
    .await;
    let mut server = mcp.config("fixture", hanafubuki_agent_mcp::McpExposurePolicy::Direct);
    server.call_timeout_seconds = 1;
    let (service, paths) = pipeline_service_with("pipeline-timeout", &model, server).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    mcp.hold.arm();
    let outcome = service.start_turn(session.id, "Go.".into()).await.unwrap();
    assert_eq!(outcome.final_answer, "Recovered.");
    let outputs = call_outputs(&model.requests()[1].body);
    assert_eq!(outputs[0].0, "call-1");
    let output = outputs[0].1.to_string();
    assert!(
        output.contains("error") && output.contains("timed out"),
        "{output}"
    );
    finish(service, paths);
}

/// A panic in one server's call is that call's error and that server's
/// fault: the server is `Failed` and reconnected, while another server's
/// call in the same step succeeds and the `mcp` plugin keeps running.
#[tokio::test]
async fn a_panicking_mcp_call_fails_its_call_and_its_server_only() {
    let echo = || {
        vec![annotated_tool(
            "echo",
            "Echo the value.",
            rmcp::model::ToolAnnotations::new()
                .read_only(false)
                .destructive(false),
        )]
    };
    let mcp = ScriptedMcpServers::start(std::collections::BTreeMap::from([
        ("panicky", echo()),
        ("fixture", echo()),
    ]))
    .await;
    let model = FakeServer::start(vec![
        (
            calls_events(&[
                ("call-1", Some("mcp__panicky"), "echo", "{\"value\":\"a\"}"),
                ("call-2", Some("mcp__fixture"), "echo", "{\"value\":\"b\"}"),
            ]),
            Duration::ZERO,
        ),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = pipeline_service("pipeline-panic", &model, &mcp).await;
    let revision = service.mcp_snapshot().await.unwrap().settings.revision;
    let panicky = mcp.config("panicky", hanafubuki_agent_mcp::McpExposurePolicy::Direct);
    hanafubuki_agent_mcp::test_support::panic_on_call(&panicky.id, "echo");
    service
        .mcp_write_server(McpServerWriteRequest {
            expected_revision: revision,
            server: panicky.clone(),
        })
        .await
        .unwrap();
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();

    let outcome = service.start_turn(session.id, "Go.".into()).await.unwrap();
    assert_eq!(outcome.final_answer, "Done.");
    let outputs = call_outputs(&model.requests()[1].body);
    assert_eq!(
        outputs[0],
        (
            "call-1".to_string(),
            Value::String("{\"error\":\"the tool failed with an internal error\"}".into())
        )
    );
    assert!(
        outputs[1].1.to_string().contains("echo called with"),
        "{outputs:?}"
    );

    // The server may already be reconnecting under load, so its failure is
    // read from the diagnostics, which keep earlier attempts' entries.
    let status = service.mcp_local_status(true).await.unwrap();
    let panicky_status = status
        .servers
        .iter()
        .find(|server| server.server_id.as_str() == "panicky")
        .unwrap();
    assert!(
        panicky_status
            .diagnostics
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.code == "tool_call_panicked"
                && entry.message
                    == "the MCP client panicked during a tool call: injected MCP client panic"),
        "{panicky_status:?}"
    );
    assert_eq!(
        mcp_status(&service, "fixture").await.phase,
        hanafubuki_agent_mcp::McpConnectionPhase::Ready
    );
    let mcp_plugin = service
        .plugin_status()
        .into_iter()
        .find(|status| status.name == "mcp")
        .unwrap();
    assert_eq!(mcp_plugin.state, super::host::AgentPluginState::Active);
    // The policy's first reconnect comes after 2 s.
    tokio::time::timeout(Duration::from_secs(30), async {
        while mcp_status(&service, "panicky").await.phase
            != hanafubuki_agent_mcp::McpConnectionPhase::Ready
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the server was reconnected");
    finish(service, paths);
}

/// A call of a tool the step does not offer gets a model-visible error and
/// the turn continues.
#[tokio::test]
async fn an_unknown_tool_gets_an_error_output_and_the_turn_continues() {
    let model = FakeServer::start(vec![
        (
            calls_events(&[("call-1", None, "no_such_tool", "{}")]),
            Duration::ZERO,
        ),
        (assistant_events("Recovered."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("pipeline-unknown", &model.base_url);
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let outcome = service.start_turn(session.id, "Go.".into()).await.unwrap();
    assert_eq!(outcome.final_answer, "Recovered.");
    assert_eq!(
        call_outputs(&model.requests()[1].body),
        [(
            "call-1".to_string(),
            Value::String(
                "{\"error\":\"no tool named `no_such_tool` is available in this step\"}".into()
            )
        )]
    );
    finish(service, paths);
}

/// Arguments that are not JSON, and JSON of the wrong type for a registered
/// and for an MCP tool, each get a model-visible error in one step and the
/// turn continues.
#[tokio::test]
async fn bad_arguments_get_error_outputs_and_the_turn_continues() {
    let mcp = echo_server().await;
    let model = FakeServer::start(vec![
        (
            calls_events(&[
                ("call-1", None, "skills_list", "{not json"),
                ("call-2", None, "skills_list", "{\"cursor\":5}"),
                ("call-3", Some("mcp__fixture"), "echo", "{\"value\":5}"),
            ]),
            Duration::ZERO,
        ),
        (assistant_events("Recovered."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = pipeline_service("pipeline-arguments", &model, &mcp).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let outcome = service.start_turn(session.id, "Go.".into()).await.unwrap();
    assert_eq!(outcome.final_answer, "Recovered.");
    let outputs = call_outputs(&model.requests()[1].body)
        .into_iter()
        .map(|(id, output)| (id, output.as_str().unwrap().to_string()))
        .collect::<Vec<_>>();
    assert_eq!(
        outputs,
        [
            ("call-1".to_string(), "{\"error\":\"the tool arguments could not be used: key must be a string at line 1 column 2\",\"receivedArguments\":\"{not json\",\"retry\":\"call the tool again with one JSON object that matches its schema\",\"tool\":\"skills_list\"}".to_string()),
            // The tool's own check, naming the argument.
            ("call-2".to_string(), "{\"error\":\"invalid tool argument cursor: invalid type: integer `5`, expected a string\"}".to_string()),
            // The kernel's schema check, for sourced tools only.
            ("call-3".to_string(), "{\"error\":\"dynamic tool arguments are invalid: arguments.value has the wrong JSON type\"}".to_string()),
        ]
    );
    finish(service, paths);
}

/// A call the model makes inside the MCP server's namespace resolves
/// through the request's wire names and runs; the history replays it with
/// its namespace. The same tool's flattened name, or a name the namespace
/// does not hold, is an unknown tool: the decoder neither joins nor splits.
#[tokio::test]
async fn a_namespaced_mcp_call_resolves_through_the_wire_names_only() {
    let mcp = echo_server().await;
    let model = FakeServer::start(vec![
        (
            calls_events(&[
                ("call-1", Some("mcp__fixture"), "echo", "{\"value\":\"hi\"}"),
                ("call-2", None, "mcp__fixture__echo", "{\"value\":\"hi\"}"),
                ("call-3", Some("mcp__fixture"), "missing", "{}"),
            ]),
            Duration::ZERO,
        ),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = pipeline_service("pipeline-namespace", &model, &mcp).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let outcome = service.start_turn(session.id, "Go.".into()).await.unwrap();
    assert_eq!(outcome.final_answer, "Done.");

    let first: Value = serde_json::from_slice(&model.requests()[0].body).unwrap();
    assert!(first["tools"].as_array().unwrap().iter().any(|tool| {
        tool["type"] == "namespace"
            && tool["name"] == "mcp__fixture"
            && tool["tools"][0]["name"] == "echo"
    }));
    let outputs = call_outputs(&model.requests()[1].body);
    assert!(outputs[0].1.to_string().contains("hi"), "{outputs:?}");
    assert!(!outputs[0].1.to_string().contains("error"), "{outputs:?}");
    assert_eq!(
        outputs[1].1,
        Value::String(
            "{\"error\":\"no tool named `mcp__fixture__echo` is available in this step\"}".into()
        )
    );
    assert_eq!(
        outputs[2].1,
        Value::String("{\"error\":\"no tool named `missing` is available in this step\"}".into())
    );
    let second: Value = serde_json::from_slice(&model.requests()[1].body).unwrap();
    let replayed = second["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call")
        .map(|item| (item["namespace"].clone(), item["name"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        replayed,
        [
            (json!("mcp__fixture"), json!("echo")),
            (Value::Null, json!("mcp__fixture__echo")),
            (json!("mcp__fixture"), json!("missing")),
        ]
    );
    finish(service, paths);
}

/// MCP annotations decide what Plan mode offers: a tool whose
/// `readOnlyHint` is true and whose `destructiveHint` is not; a tool without
/// hints, or one that says it is read-only and destructive, only outside it.
#[tokio::test]
async fn plan_mode_offers_only_the_mcp_tools_annotated_read_only() {
    use rmcp::model::ToolAnnotations;
    let mcp = ScriptedMcpServers::start(std::collections::BTreeMap::from([(
        "fixture",
        vec![
            scripted_tool("reader", "Read-only, not destructive."),
            annotated_tool(
                "hinted",
                "Read-only; destructive unset.",
                ToolAnnotations::new().read_only(true),
            ),
            annotated_tool("writer", "No hints.", ToolAnnotations::new()),
            annotated_tool(
                "contradictory",
                "Read-only and destructive.",
                ToolAnnotations::new().read_only(true).destructive(true),
            ),
        ],
    )]))
    .await;
    let model = FakeServer::start(vec![
        (assistant_events("Planned."), Duration::ZERO),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = pipeline_service("pipeline-plan-mode", &model, &mcp).await;
    for mode in [AgentSessionMode::Plan, AgentSessionMode::Default] {
        let session = service.create_session(mode).await.unwrap();
        service.start_turn(session.id, "Go.".into()).await.unwrap();
    }

    let offered = |index: usize| -> Vec<String> {
        let body: Value = serde_json::from_slice(&model.requests()[index].body).unwrap();
        let mut names = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|tool| tool["name"] == "mcp__fixture")
            .flat_map(|tool| tool["tools"].as_array().unwrap().clone())
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
    };
    assert_eq!(offered(0), ["hinted", "reader"]);
    assert_eq!(offered(1), ["contradictory", "hinted", "reader", "writer"]);
    finish(service, paths);
}

/// A server whose `echo` writes without being destructive: an Exclusive
/// tool that the fixture's `Allow` policy runs without a review.
async fn echo_server() -> ScriptedMcpServers {
    ScriptedMcpServers::start(std::collections::BTreeMap::from([(
        "fixture",
        vec![annotated_tool(
            "echo",
            "Echo the value.",
            rmcp::model::ToolAnnotations::new()
                .read_only(false)
                .destructive(false),
        )],
    )]))
    .await
}

/// A service on `model` with the `fixture` server of `mcp` connected
/// (Direct, no approval).
async fn pipeline_service(
    name: &str,
    model: &FakeServer,
    mcp: &ScriptedMcpServers,
) -> (AgentApplicationService, TestPaths) {
    pipeline_service_with(
        name,
        model,
        mcp.config("fixture", hanafubuki_agent_mcp::McpExposurePolicy::Direct),
    )
    .await
}

/// A service on `model` with `server` connected.
async fn pipeline_service_with(
    name: &str,
    model: &FakeServer,
    server: hanafubuki_agent_mcp::McpServerConfig,
) -> (AgentApplicationService, TestPaths) {
    let (service, paths) = configured_service(name, &model.base_url);
    let revision = service.mcp_snapshot().await.unwrap().settings.revision;
    service
        .mcp_write_server(McpServerWriteRequest {
            expected_revision: revision,
            server,
        })
        .await
        .unwrap();
    (service, paths)
}

/// The status of MCP server `id`.
async fn mcp_status(
    service: &AgentApplicationService,
    id: &str,
) -> hanafubuki_agent_mcp::McpServerStatus {
    service
        .mcp_snapshot()
        .await
        .unwrap()
        .servers
        .into_iter()
        .find(|status| status.server_id.as_str() == id)
        .unwrap()
}

fn spawn_turn(
    service: &AgentApplicationService,
    session_id: hanafubuki_agent_kernel::AgentSessionId,
) -> tokio::task::JoinHandle<
    Result<hanafubuki_agent_kernel::AgentTurnOutcome, hanafubuki_agent_kernel::AgentError>,
> {
    let service = service.clone();
    tokio::spawn(async move { service.start_turn(session_id, "Go.".into()).await })
}

/// Waits for the tool event `event` (as `tool_events_of` writes it).
async fn wait_for_event(
    service: &AgentApplicationService,
    session_id: hanafubuki_agent_kernel::AgentSessionId,
    event: &str,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !tool_events_of(service, session_id)
            .iter()
            .any(|seen| seen == event)
        {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no {event}"));
}

fn approval_requests(
    service: &AgentApplicationService,
    session_id: hanafubuki_agent_kernel::AgentSessionId,
) -> Vec<hanafubuki_agent_approval::ApprovalRequest> {
    service
        .events_since(session_id, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::ApprovalRequested { request } => Some(request),
            _ => None,
        })
        .collect()
}

/// The session's tool events as `started|finished|completed <call id>`.
fn tool_events_of(
    service: &AgentApplicationService,
    session_id: hanafubuki_agent_kernel::AgentSessionId,
) -> Vec<String> {
    service
        .events_since(session_id, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::ToolStarted { call_id, .. } => Some(format!("started {call_id}")),
            AgentEventKind::ToolExecutionFinished { call_id, .. } => {
                Some(format!("finished {call_id}"))
            }
            AgentEventKind::ToolCompleted { call_id, .. } => Some(format!("completed {call_id}")),
            _ => None,
        })
        .collect()
}

/// The `function_call_output` items of a request body, in order.
fn call_outputs(body: &[u8]) -> Vec<(String, Value)> {
    let request: Value = serde_json::from_slice(body).unwrap();
    request["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .map(|item| {
            (
                item["call_id"].as_str().unwrap().to_string(),
                item["output"].clone(),
            )
        })
        .collect()
}

/// One Responses stream whose output is the function calls `(call id,
/// namespace, name, arguments)`.
fn calls_events(calls: &[(&str, Option<&str>, &str, &str)]) -> Vec<u8> {
    let mut events = String::from(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-calls\"}}\n\n",
    );
    for (index, (call_id, namespace, name, arguments)) in calls.iter().enumerate() {
        let mut item = json!({
            "type": "function_call",
            "id": format!("item-{index}"),
            "call_id": call_id,
            "name": name,
            "arguments": arguments,
        });
        if let Some(namespace) = namespace {
            item["namespace"] = json!(namespace);
        }
        let added = json!({"type": "response.output_item.added", "item": item});
        let done = json!({"type": "response.output_item.done", "item": item});
        events.push_str(&format!("data: {added}\n\ndata: {done}\n\n"));
    }
    events.push_str(
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-calls\",\"end_turn\":false}}\n\n",
    );
    events.into_bytes()
}

fn finish(service: AgentApplicationService, paths: TestPaths) {
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
