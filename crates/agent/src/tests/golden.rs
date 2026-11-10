//! Golden baselines for the Agent over its own plugin set, with a host that
//! adds no plugin rows, context or instructions
//! (site/content/docs/plugin-rules.en.mdx,
//! "Testing a plugin").
//!
//! They pin what the model receives (instructions, tool catalog, input items),
//! the presentation event sequence of a tool round trip, and that history
//! written by an earlier build still opens and continues. Rerun with
//! `HANAFUBUKI_AGENT_UPDATE_GOLDEN=1` to rewrite the fixtures after an intentional change,
//! then review the fixture diff. The history corpus is re-recorded only with
//! `HANAFUBUKI_AGENT_UPDATE_GOLDEN_HISTORY=1`, after an intentional history format change:
//! re-recording it would stop proving that older history still opens.
//!
//! The fixtures are Linux-only: the world state names the platform and the
//! architecture, and temporary paths differ per platform.

use std::path::{Path, PathBuf};

use super::*;
use crate::{
    AgentHistorySortDirection, AgentSessionListRequest, AgentSessionOrigin,
    AgentSessionParentFilter, AgentTranscriptPageRequest, McpServerWriteRequest,
};
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentGoalOrigin, AgentGoalStatus,
    AgentSessionPersistence,
};
use serde_json::{Map, Value};

const GOLDEN_DIR: &str = "src/tests/golden";

#[tokio::test]
async fn default_mode_request_matches_golden() {
    let body = first_request_with(
        "golden-default",
        AgentSessionMode::Default,
        |_, _| {},
        |_, _| {},
    )
    .await;
    assert_golden("default_mode_request", &body);
}

#[tokio::test]
async fn plan_mode_request_matches_golden() {
    let body =
        first_request_with("golden-plan", AgentSessionMode::Plan, |_, _| {}, |_, _| {}).await;
    assert_golden("plan_mode_request", &body);
}

#[tokio::test]
async fn tool_round_trip_matches_golden() {
    let (requests, kinds) = run_tool_round_trip(
        "golden-round-trip",
        [
            tool_events("skills_list", "{}"),
            assistant_events(SKILLS_ANSWER),
        ],
        |_| {},
    )
    .await;
    assert_golden("tool_round_trip_request", &requests);
    assert_golden("tool_round_trip_events", &kinds);
}

const SKILLS_ANSWER: &str = "No Skills are installed.";

/// Runs one `skills_list` round trip over the two model `responses` (the call,
/// then the answer [`SKILLS_ANSWER`]) and returns the normalized second
/// request and the presentation event kinds. `after_shutdown` sees the paths
/// once the history is flushed.
async fn run_tool_round_trip(
    name: &str,
    responses: [Vec<u8>; 2],
    after_shutdown: impl FnOnce(&TestPaths),
) -> (Value, Value) {
    let server = FakeServer::start(
        responses
            .into_iter()
            .map(|response| (response, Duration::ZERO))
            .collect(),
    )
    .await;
    let (service, paths) = configured_service(name, &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let outcome = service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, SKILLS_ANSWER);

    let requests = server.requests();
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    let second = normalize(&second, &paths.app_data_dir);

    let kinds = service
        .events_since(session.id, None)
        .unwrap()
        .events
        .iter()
        .map(|event| serde_json::to_value(&event.kind).unwrap()["type"].clone())
        .collect::<Vec<_>>();

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    after_shutdown(&paths);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
    (second, Value::Array(kinds))
}

/// History written by the build that recorded the fixture must open, keep its
/// transcript, and accept a new turn.
#[tokio::test]
async fn recorded_history_reopens_and_continues() {
    if updating_history() {
        run_tool_round_trip(
            "golden-history-corpus",
            [
                tool_events("skills_list", "{}"),
                assistant_events(SKILLS_ANSWER),
            ],
            |paths| record_history_fixture(paths, "history"),
        )
        .await;
    }
    reopen_and_continue("history", "recorded_history_transcript", "List the tasks.").await;
}

/// Reasoning items written by the build that recorded the fixture open with
/// their text and stay out of the next request (`golden/history-reasoning/`,
/// which Hanafubuki's integration mirror holds the same bytes of).
#[tokio::test]
async fn recorded_reasoning_history_reopens_and_continues() {
    if updating_history() {
        run_tool_round_trip(
            "golden-reasoning-corpus",
            [
                reasoning_tool_events(RECORDED_TOOL_THINKING, "skills_list", "{}"),
                reasoning_answer_events(RECORDED_ANSWER_THINKING, SKILLS_ANSWER),
            ],
            |paths| record_history_fixture(paths, "history-reasoning"),
        )
        .await;
    }
    let request = reopen_and_continue(
        "history-reasoning",
        "recorded_reasoning_history_transcript",
        "List the Skills.",
    )
    .await;
    assert!(request.contains(SKILLS_ANSWER));
    for thinking in [RECORDED_TOOL_THINKING, RECORDED_ANSWER_THINKING] {
        assert!(!request.contains(thinking), "thinking is never model input");
    }
}

const RECORDED_TOOL_THINKING: &str = "**Listing Skills**\n\nThe catalog says which are installed.";
const RECORDED_ANSWER_THINKING: &str = "None are installed, so say so.";

/// Opens the one session of the `corpus` history, compares its transcript
/// with the `transcript` golden, runs a turn in it, and returns the body of
/// that turn's request. `recorded_prompt` is the recorded turn's prompt.
async fn reopen_and_continue(corpus: &str, transcript: &str, recorded_prompt: &str) -> String {
    let server = FakeServer::start(vec![(assistant_events("Continued."), Duration::ZERO)]).await;
    let (service, paths) = service_with(&format!("golden-{corpus}"), |paths, config| {
        copy_tree(&golden_path(corpus), &agent_history_dir(paths));
        configure_test_provider(config, &server.base_url, None);
    });
    let sessions = service
        .list_sessions_page(AgentSessionListRequest::default())
        .unwrap()
        .sessions;
    assert_eq!(sessions.len(), 1, "the corpus holds exactly one session");
    let session_id = sessions[0].session_id;

    let opened = service.open_session(session_id, 50).unwrap();
    assert_golden(
        transcript,
        &transcript_digest(&serde_json::to_value(&opened.transcript).unwrap()),
    );

    let outcome = service
        .start_turn(session_id, "Continue.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Continued.");
    let page = service
        .read_transcript_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 50,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    let rendered = serde_json::to_string(&page).unwrap();
    assert!(rendered.contains(recorded_prompt));
    assert!(rendered.contains("Continued."));
    let request = String::from_utf8(server.requests()[0].body.clone()).unwrap();

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
    request
}

/// A child session written by the build that recorded the fixture opens
/// under its parent, keeps its lineage and the workspace it was created in,
/// and accepts a new turn (`golden/history-lineage/`: a parent and a
/// subagent child forked from its head, each with one turn).
#[tokio::test]
async fn recorded_lineage_history_reopens_and_continues() {
    if updating_history() {
        record_lineage_history().await;
    }
    let server = FakeServer::start(vec![(assistant_events("Continued."), Duration::ZERO)]).await;
    let (service, paths) = service_with("golden-history-lineage", |paths, config| {
        copy_tree(&golden_path("history-lineage"), &agent_history_dir(paths));
        configure_test_provider(config, &server.base_url, None);
    });
    let listed = |parent| {
        service
            .list_sessions_page(AgentSessionListRequest {
                parent,
                ..AgentSessionListRequest::default()
            })
            .unwrap()
            .sessions
    };
    let parents = listed(AgentSessionParentFilter::TopLevel);
    assert_eq!(
        parents.len(),
        1,
        "the child is not listed beside its parent"
    );
    let parent = &parents[0];
    assert_eq!((parent.parent_session_id, parent.origin), (None, None));
    assert_eq!(parent.branch.as_deref(), Some("main"));
    assert!(parent.cwd.is_some());
    let children = listed(AgentSessionParentFilter::ChildrenOf(parent.session_id));
    assert_eq!(children.len(), 1);
    let child = &children[0];
    assert_eq!(child.parent_session_id, Some(parent.session_id));
    assert_eq!(child.origin, Some(AgentSessionOrigin::Subagent));
    assert_eq!(child.forked_from_session_id, Some(parent.session_id));
    assert_eq!((&child.cwd, &child.branch), (&parent.cwd, &parent.branch));

    let opened = service.open_session(child.session_id, 50).unwrap();
    assert_golden(
        "recorded_lineage_history_transcript",
        &transcript_digest(&serde_json::to_value(&opened.transcript).unwrap()),
    );
    let outcome = service
        .start_turn(child.session_id, "Continue.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Continued.");
    let request = String::from_utf8(server.requests()[0].body.clone()).unwrap();
    assert!(request.contains("Parent question.") && request.contains("Child question."));

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Records `golden/history-lineage/`, its workspace on the branch `main`.
async fn record_lineage_history() {
    let server = FakeServer::start(vec![
        (assistant_events("Parent answer."), Duration::ZERO),
        (assistant_events("Child answer."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = service_with("golden-lineage-corpus", |paths, config| {
        configure_test_provider(config, &server.base_url, None);
        let head = paths.managed_instances_dir.join(".git").join("HEAD");
        std::fs::create_dir_all(head.parent().unwrap()).unwrap();
        std::fs::write(head, "ref: refs/heads/main\n").unwrap();
    });
    let parent = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(parent.id, "Parent question.".to_string().into())
        .await
        .unwrap();
    let driver = service.test_session_driver().clone();
    let child = driver
        .create_child(AgentChildSessionRequest {
            parent_session_id: parent.id,
            origin: AgentSessionOrigin::Subagent,
            persistence: AgentSessionPersistence::Durable,
            base: AgentChildSessionBase::ParentHead,
            context: Vec::new(),
        })
        .unwrap();
    driver
        .run_turn(child.id, "Child question.".into())
        .await
        .unwrap();
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    record_history_fixture(&paths, "history-lineage");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A session goal written by the build that recorded the fixture reads back
/// with its status and usage, and the session accepts a new turn, which a
/// complete goal does not continue (`golden/history-goal/`: the model
/// created a goal, was kept going once, and completed it).
#[tokio::test]
async fn recorded_goal_history_reopens_and_continues() {
    if updating_history() {
        record_goal_history().await;
    }
    let server = FakeServer::start(vec![(assistant_events("Continued."), Duration::ZERO)]).await;
    let (service, paths) = service_with("golden-history-goal", |paths, config| {
        copy_tree(&golden_path("history-goal"), &agent_history_dir(paths));
        configure_test_provider(config, &server.base_url, None);
    });
    let sessions = service
        .list_sessions_page(AgentSessionListRequest::default())
        .unwrap()
        .sessions;
    assert_eq!(sessions.len(), 1, "the corpus holds exactly one session");
    let session_id = sessions[0].session_id;

    let goal = service.goal(session_id).await.unwrap().unwrap();
    assert_eq!(goal.objective, "Write the changelog");
    assert_eq!(goal.status, AgentGoalStatus::Complete);
    assert_eq!(goal.origin, AgentGoalOrigin::Model);
    assert_eq!(goal.token_budget, Some(100_000));
    assert!(goal.tokens_used > 0 && goal.created_at > 0);
    let opened = service.open_session(session_id, 50).unwrap();
    assert_golden(
        "recorded_goal_history_transcript",
        &transcript_digest(&serde_json::to_value(&opened.transcript).unwrap()),
    );
    let outcome = service
        .start_turn(session_id, "Continue.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Continued.");
    assert_eq!(server.requests().len(), 1);
    let request = String::from_utf8(server.requests()[0].body.clone()).unwrap();
    assert!(request.contains("Continue working toward the active session goal."));
    assert!(request.contains(r#"Goal status: complete."#));
    assert!(request.contains(r#"set the goal: \"Write the changelog\""#));

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Records `golden/history-goal/`.
async fn record_goal_history() {
    let server = FakeServer::start(vec![
        (
            tool_events(
                "create_goal",
                r#"{"objective":"Write the changelog","token_budget":100000}"#,
            ),
            Duration::ZERO,
        ),
        (assistant_events("Goal set."), Duration::ZERO),
        (
            tool_events("update_goal", r#"{"status":"complete"}"#),
            Duration::ZERO,
        ),
        (assistant_events("Changelog written."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("golden-goal-corpus", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(
            session.id,
            "Make writing the changelog a goal.".to_string().into(),
        )
        .await
        .unwrap();
    assert_eq!(server.requests().len(), 4);
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    record_history_fixture(&paths, "history-goal");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The side policy and the boundary follow the history a side conversation
/// inherits, before its first message.
#[tokio::test]
async fn side_conversation_request_matches_golden() {
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 2]).await;
    let (service, paths) = configured_service("golden-side", &server.base_url);
    let parent = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(parent.id, "Hello.".to_string().into())
        .await
        .unwrap();
    let side = service.open_side_conversation(parent.id).unwrap();
    service
        .start_turn(side.id, "What did you just say?".to_string().into())
        .await
        .unwrap();

    let body: Value = serde_json::from_slice(&server.requests()[1].body).unwrap();
    assert_golden(
        "side_conversation_request",
        &normalize(&body, &paths.app_data_dir),
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A review session's first request: the rubric among the capabilities and
/// the review's request as its only message, nothing of its parent's.
#[tokio::test]
async fn review_request_matches_golden() {
    let server =
        FakeServer::start(vec![(assistant_events("No findings."), Duration::ZERO); 2]).await;
    let (service, paths) = configured_service("golden-review", &server.base_url);
    let parent = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(parent.id, "Hello.".to_string().into())
        .await
        .unwrap();
    service
        .start_review(
            parent.id,
            crate::AgentReviewTarget::Custom {
                instructions: "Review the error handling in src/lib.rs.".to_string(),
            },
        )
        .await
        .unwrap();

    let body: Value = serde_json::from_slice(&server.requests()[1].body).unwrap();
    assert_golden("review_request", &normalize(&body, &paths.app_data_dir));
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn skill_catalog_request_matches_golden() {
    let body = first_request_with(
        "golden-skills",
        AgentSessionMode::Default,
        |_, _| {},
        enable_two_skills,
    )
    .await;
    assert_golden("skill_catalog_request", &body);
}

/// An installed Claude Code plugin's Skill, listed under its namespace, and
/// the plugin itself in the "available plugins" section, with its MCP
/// server (which never connects).
#[tokio::test]
async fn installed_plugin_request_matches_golden() {
    let body = first_request_with(
        "golden-installed-plugin",
        AgentSessionMode::Default,
        |paths, _| {
            let source = super::packages::claude_plugin(
                paths,
                "demo",
                serde_json::json!({ "files": { "command": "hanafubuki-golden-missing-server" } }),
                None,
            );
            super::packages::install(paths, &source, "demo@local");
        },
        |_, _| {},
    )
    .await;
    assert_golden("installed_plugin_request", &body);
}

#[tokio::test]
async fn workspace_instructions_request_matches_golden() {
    let body = first_request_with(
        "golden-workspace-instructions",
        AgentSessionMode::Default,
        |_, _| {},
        |_, paths| {
            std::fs::write(
                paths.managed_instances_dir.join("AGENTS.md"),
                "# Workspace rules\n\nRun the narrow checks first.\n",
            )
            .unwrap();
        },
    )
    .await;
    assert_golden("workspace_instructions_request", &body);
}

#[tokio::test]
async fn memory_summary_request_matches_golden() {
    let body = first_request_with(
        "golden-memory",
        AgentSessionMode::Default,
        |_, config| enable_memory(config, false),
        |service, _| {
            let memory = service.test_memory();
            let artifacts = &memory.repository().paths().artifacts;
            std::fs::write(
                artifacts.join("memory_summary.md"),
                "v1\n\n- Validation preferences: MEMORY.md\n",
            )
            .unwrap();
        },
    )
    .await;
    assert_golden("memory_summary_request", &body);
}

/// MCP tools as the model receives them, from three scripted servers:
/// `fixture` (Direct) with a plain name, a 64-byte name, a name the wire
/// table fits to the 128-byte Responses limit, a name with an illegal
/// character and `b__c`; `fixture__b` (Direct) whose `c` flattens to the
/// same `mcp__fixture__b__c`, so it is renamed and the rename reported; and
/// `deferred` (Deferred), reached only through `tool_search`.
#[tokio::test]
async fn mcp_tools_request_matches_golden() {
    let mcp = scripted_mcp_servers().await;
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO)]).await;
    let (service, paths) = configured_service("golden-mcp", &server.base_url);
    connect_scripted_mcp_servers(&service, &mcp).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(session.id, "Hello.".to_string().into())
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let normalized = normalize(&body, &paths.app_data_dir);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
    assert_golden("mcp_tools_request", &normalized);
}

/// A `tool_search` that finds the deferred MCP tool, then a call of it: the
/// request after the search and the request after the call.
#[tokio::test]
async fn tool_search_round_trip_matches_golden() {
    let mcp = scripted_mcp_servers().await;
    let server = FakeServer::start(vec![
        (tool_search_events("look up a record"), Duration::ZERO),
        (
            namespace_tool_events("mcp__deferred", "lookup", "{\"value\":\"42\"}"),
            Duration::ZERO,
        ),
        (assistant_events("Found it."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("golden-tool-search", &server.base_url);
    connect_scripted_mcp_servers(&service, &mcp).await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let outcome = service
        .start_turn(session.id, "Look up record 42.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Found it.");
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    let body = |index: usize| -> Value { serde_json::from_slice(&requests[index].body).unwrap() };
    let pinned = serde_json::json!({
        "after_search": body(1),
        "after_call": body(2),
    });
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let normalized = normalize(&pinned, &paths.app_data_dir);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
    assert_golden("tool_search_round_trip", &normalized);
}

async fn scripted_mcp_servers() -> ScriptedMcpServers {
    ScriptedMcpServers::start(std::collections::BTreeMap::from([
        (
            "fixture",
            vec![
                scripted_tool("echo", "Echo the value."),
                scripted_tool(&format!("long_{}", "x".repeat(59)), "A 64-byte name."),
                scripted_tool(&"y".repeat(120), "A name over the flattened limit."),
                scripted_tool("my.tool", "A name with a dot."),
                scripted_tool("b__c", "Flattens like fixture__b's c."),
            ],
        ),
        (
            "fixture__b",
            vec![scripted_tool("c", "Flattens like fixture's b__c.")],
        ),
        (
            "deferred",
            vec![scripted_tool("lookup", "Look up a record.")],
        ),
    ]))
    .await
}

async fn connect_scripted_mcp_servers(service: &AgentApplicationService, mcp: &ScriptedMcpServers) {
    use McpExposurePolicy::{Deferred, Direct};
    let mut revision = service.mcp_snapshot().await.unwrap().settings.revision;
    for (id, exposure) in [
        ("fixture", Direct),
        ("fixture__b", Direct),
        ("deferred", Deferred),
    ] {
        let snapshot = service
            .mcp_write_server(McpServerWriteRequest {
                expected_revision: revision,
                server: mcp.config(id, exposure),
            })
            .await
            .unwrap();
        revision = snapshot.settings.revision;
    }
}

/// The first request of a "Hello." turn in `mode`, after `configure` sets
/// the settings and `prepare` the fixture files of a configured service.
async fn first_request_with(
    name: &str,
    mode: AgentSessionMode,
    configure: impl FnOnce(&TestPaths, &AgentConfig),
    prepare: impl FnOnce(&AgentApplicationService, &TestPaths),
) -> Value {
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO)]).await;
    let (service, paths) = service_with(name, |paths, config| {
        configure_test_provider(config, &server.base_url, None);
        configure(paths, config);
    });
    prepare(&service, &paths);
    let session = service.create_session(mode).await.unwrap();
    service
        .start_turn(session.id, "Hello.".to_string().into())
        .await
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let normalized = normalize(&body, &paths.app_data_dir);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
    normalized
}

/// Keeps the item kinds and the visible text of a transcript page; ids,
/// revisions and timestamps belong to the recording run.
fn transcript_digest(page: &Value) -> Value {
    fn texts(value: &Value, out: &mut Vec<Value>) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    match (key.as_str(), value) {
                        ("type" | "kind" | "text" | "role" | "name", Value::String(_)) => {
                            out.push(Value::String(format!("{key}={}", value.as_str().unwrap())));
                        }
                        _ => texts(value, out),
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| texts(item, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    texts(&sorted(page), &mut out);
    Value::Array(out)
}

pub(super) fn normalize(value: &Value, app_data_dir: &Path) -> Value {
    let app_data_dir = app_data_dir.to_string_lossy().into_owned();
    let replace = |text: &str| {
        let text = text.replace(&app_data_dir, "<APP_DATA>").replace(
            &format!("\"architecture\":\"{}\"", std::env::consts::ARCH),
            "\"architecture\":\"<ARCH>\"",
        );
        redact_digests(&text)
    };
    fn walk(value: &Value, replace: &dyn Fn(&str) -> String) -> Value {
        match value {
            Value::String(text) => Value::String(replace(text)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| walk(item, replace)).collect())
            }
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), walk(value, replace)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    sorted(&walk(value, &replace))
}

/// World-state sections carry a content digest that covers temporary paths,
/// and the tool catalog revision covers each MCP connection's generation.
fn redact_digests(text: &str) -> String {
    ["version=\"", "\"catalogRevision\":\""]
        .into_iter()
        .fold(text.to_string(), |text, marker| {
            redact_digests_after(&text, marker)
        })
}

fn redact_digests_after(text: &str, marker: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find(marker) {
        let start = position + marker.len();
        out.push_str(&rest[..start]);
        let candidate = &rest[start..];
        let digest_length = candidate
            .bytes()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        if digest_length == 64 {
            out.push_str("<DIGEST>");
            rest = &candidate[64..];
        } else {
            rest = candidate;
        }
    }
    out.push_str(rest);
    out
}

/// Object keys in a stable order whether or not `serde_json/preserve_order` is
/// enabled somewhere in the dependency graph.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), sorted(&object[key]));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn updating() -> bool {
    std::env::var_os("HANAFUBUKI_AGENT_UPDATE_GOLDEN").is_some()
}

fn updating_history() -> bool {
    std::env::var_os("HANAFUBUKI_AGENT_UPDATE_GOLDEN_HISTORY").is_some()
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(GOLDEN_DIR)
        .join(name)
}

pub(super) fn assert_golden(name: &str, actual: &Value) {
    let path = golden_path(&format!("{name}.json"));
    let rendered = format!("{}\n", serde_json::to_string_pretty(actual).unwrap());
    if updating() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, rendered).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing golden {}: {error}", path.display()));
    assert!(
        expected == rendered,
        "golden {name} changed; rerun with HANAFUBUKI_AGENT_UPDATE_GOLDEN=1 and review the fixture diff"
    );
}

pub(super) fn agent_history_dir(paths: &TestPaths) -> PathBuf {
    paths.data_dir.join("agent-test").join("history")
}

/// Stores the session JSONL written by this build as the `corpus` fixture.
/// The SQLite index is a projection and is rebuilt on open, so the corpus
/// leaves it out.
fn record_history_fixture(paths: &TestPaths, corpus: &str) {
    let target = golden_path(corpus);
    let _ = std::fs::remove_dir_all(&target);
    copy_tree(&agent_history_dir(paths), &target);
}

pub(super) fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("index.sqlite3") {
            continue;
        }
        let destination = target.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), destination).unwrap();
        }
    }
}
