//! Reasoning against a fake Provider, through the real transports: a model
//! that takes manual thinking gets the turn's signed thinking block back
//! with the tool call it preceded (the fake refuses the continuation
//! otherwise, as Anthropic does), and no earlier turn's; DeepSeek's
//! thinking mode gets every earlier assistant message back with its
//! `reasoning_content`, from every turn and after a restart (the fake
//! refuses a request with tools otherwise, as DeepSeek does); a Provider
//! that refuses the reasoning parameter in one run is sent none in the next
//! (`model-metadata.json`).

use hanafubuki_agent_kernel::{AgentSessionMode, ModelReasoningEffort};
use hanafubuki_agent_process::executor::ExecutorManager;
use serde_json::{json, Value};

use super::*;
use crate::test_support::TestPaths;

/// OpenAI's refusal of `reasoning.effort` (openai/openai-python#2532).
const REASONING_REFUSAL: &str = r#"{"error":{"message":"Unsupported parameter: 'reasoning.effort' is not supported with this model.","type":"invalid_request_error","param":"reasoning.effort","code":"unsupported_parameter"}}"#;

fn body(request: &CapturedRequest) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}

/// Whether a file under `directory` holds `needle`.
fn stored_anywhere(directory: &std::path::Path, needle: &[u8]) -> bool {
    std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let path = entry.path();
            if path.is_dir() {
                return stored_anywhere(&path, needle);
            }
            std::fs::read(&path)
                .is_ok_and(|bytes| bytes.windows(needle.len()).any(|window| window == needle))
        })
}

fn stop(service: &AgentApplicationService) {
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
}

/// A new Agent over `paths`' storage, as after a restart.
fn reopen(config: AgentConfig, paths: &TestPaths) -> AgentApplicationService {
    AgentApplicationService::new_with_host(
        config,
        test_host(paths),
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_thinking_sends_the_turns_signed_block_back_with_its_tool_call() {
    let server = FakeServer::start(vec![
        (
            anthropic_thinking_tool_events("List the Skills first.", "sig-1", "skills_list"),
            Duration::ZERO,
        ),
        (anthropic_assistant_events("Listed."), Duration::ZERO),
        (anthropic_assistant_events("Again."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = service_with("reasoning-manual-thinking", |_, config| {
        configure_test_provider(config, &server.base_url, Some(ModelReasoningEffort::Low));
        config
            .mutate_providers(1, |registry| {
                registry.providers[0].protocol = AgentModelProtocol::AnthropicMessages;
                registry.model_policy.model = "claude-sonnet-4-5".into();
                Ok(())
            })
            .unwrap();
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let outcome = service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Listed.");
    service
        .start_turn(session.id, "And again.".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 3, "no request was refused");
    let (first, continuation, next_turn) =
        (body(&requests[0]), body(&requests[1]), body(&requests[2]));
    for request in [&first, &continuation, &next_turn] {
        assert_eq!(
            request["thinking"],
            json!({"type": "enabled", "budget_tokens": 4096})
        );
    }
    let assistant = |request: &Value| -> Vec<Value> {
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "assistant")
            .flat_map(|message| message["content"].as_array().unwrap().clone())
            .collect()
    };
    let sent_back = assistant(&continuation);
    assert_eq!(
        sent_back[0],
        json!({"type": "thinking", "thinking": "List the Skills first.", "signature": "sig-1"})
    );
    assert_eq!(sent_back[1]["type"], "tool_use");
    assert!(
        assistant(&next_turn)
            .iter()
            .all(|block| block["type"] != "thinking"),
        "an earlier turn's block is not sent back"
    );
    stop(&service);
    assert!(
        !stored_anywhere(&paths.app_data_dir, b"sig-1"),
        "the signature is never persisted"
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The Provider refuses the reasoning parameter: the turn goes on without
/// it, and the next run, over the same data directory, sends that model
/// none from a new session's first request.
#[tokio::test]
async fn a_reasoning_refusal_in_one_run_is_not_sent_in_the_next() {
    let server = FakeServer::start(vec![
        (http_error(400, REASONING_REFUSAL), Duration::ZERO),
        (assistant_events("Thought less."), Duration::ZERO),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let (service, paths, config) = service_with_config("reasoning-refusal-restart", |_, config| {
        configure_test_provider(config, &server.base_url, Some(ModelReasoningEffort::High));
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let outcome = service
        .start_turn(session.id, "think".into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Thought less.");
    let requests = server.requests();
    assert_eq!(
        body(&requests[0])["reasoning"],
        json!({"effort": "high", "summary": "auto"})
    );
    assert!(body(&requests[1]).get("reasoning").is_none());
    stop(&service);

    let reopened = reopen(config, &paths);
    let session = reopened
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    reopened
        .start_turn(session.id, "hello".into())
        .await
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 3, "no request was refused");
    assert!(body(&requests[2]).get("reasoning").is_none());
    stop(&reopened);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A Chat Completions Provider serving `model`.
fn configure_chat_provider(config: &AgentConfig, base_url: &str, model: &str) {
    configure_test_provider(config, base_url, None);
    config
        .mutate_providers(1, |registry| {
            registry.providers[0].protocol = AgentModelProtocol::ChatCompletions;
            registry.model_policy.model = model.into();
            Ok(())
        })
        .unwrap();
}

/// The `reasoning_content` of each assistant message of `request`, in
/// order (`None` for a message without one).
fn reasoning_sent(request: &Value) -> Vec<Option<String>> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "assistant")
        .map(|message| message["reasoning_content"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_reasoning_content_goes_back_from_every_turn_and_after_a_restart() {
    let thought = |text: &str| Some(text.to_owned());
    let server = FakeServer::start(vec![
        (
            chat_reasoning_tool_events(
                "reasoning_content",
                "List the Skills first.",
                "call-1",
                "skills_list",
            ),
            Duration::ZERO,
        ),
        (
            chat_reasoning_answer_events("reasoning_content", "They are listed.", "Listed."),
            Duration::ZERO,
        ),
        (
            chat_reasoning_answer_events("reasoning_content", "Just greet.", "Hello."),
            Duration::ZERO,
        ),
        (
            chat_reasoning_answer_events("reasoning_content", "Still here.", "Yes."),
            Duration::ZERO,
        ),
        (
            chat_reasoning_answer_events("reasoning_content", "Back again.", "Back."),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths, config) = service_with_config("reasoning-chat-content", |_, config| {
        configure_chat_provider(config, &server.base_url, "deepseek-v4-pro");
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    for (prompt, answer) in [
        ("List the Skills.", "Listed."),
        ("Hi.", "Hello."),
        ("Still there?", "Yes."),
    ] {
        let outcome = service
            .start_turn(session.id, prompt.to_string().into())
            .await
            .unwrap();
        assert_eq!(outcome.final_answer, answer);
    }
    stop(&service);
    assert!(
        stored_anywhere(&paths.app_data_dir, b"Just greet."),
        "the text sent back is persisted"
    );

    let reopened = reopen(config, &paths);
    let outcome = reopened
        .start_turn(session.id, "Again.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Back.");
    stop(&reopened);

    let requests = server.requests().iter().map(body).collect::<Vec<_>>();
    assert_eq!(requests.len(), 5, "no request was refused");
    for request in &requests {
        assert!(!request["tools"].as_array().unwrap().is_empty());
    }
    // Within the tool loop.
    assert_eq!(
        reasoning_sent(&requests[1]),
        [thought("List the Skills first.")]
    );
    // A second user turn.
    assert_eq!(
        reasoning_sent(&requests[2]),
        [
            thought("List the Skills first."),
            thought("They are listed.")
        ]
    );
    // After a turn whose answer made no tool call.
    assert_eq!(
        reasoning_sent(&requests[3]),
        [
            thought("List the Skills first."),
            thought("They are listed."),
            thought("Just greet.")
        ]
    );
    // The session reopened by a new runtime over the same data directory.
    assert_eq!(
        reasoning_sent(&requests[4]),
        [
            thought("List the Skills first."),
            thought("They are listed."),
            thought("Just greet."),
            thought("Still here.")
        ]
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Reasoning that came as `reasoning` (OpenRouter's field) is shown, not
/// sent back.
#[tokio::test(flavor = "multi_thread")]
async fn chat_reasoning_that_came_as_reasoning_is_not_sent_back() {
    let server = FakeServer::start(vec![
        (
            chat_reasoning_tool_events("reasoning", "List them.", "call-1", "skills_list"),
            Duration::ZERO,
        ),
        (
            chat_reasoning_answer_events("reasoning", "Listed them.", "Listed."),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths) = service_with("reasoning-chat-openrouter", |_, config| {
        configure_chat_provider(config, &server.base_url, "vendor/thinking-model");
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let outcome = service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Listed.");
    stop(&service);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(reasoning_sent(&body(&requests[1])), [None]);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A compaction's request carries the history and the tools, so it sends
/// the reasoning back too; the turn after it is not refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_compaction_sends_chat_reasoning_content_back() {
    let server = FakeServer::start(vec![
        (
            chat_reasoning_tool_events("reasoning_content", "List first.", "call-1", "skills_list"),
            Duration::ZERO,
        ),
        (
            chat_reasoning_answer_events("reasoning_content", "Listed now.", "Listed."),
            Duration::ZERO,
        ),
        (
            chat_reasoning_answer_events(
                "reasoning_content",
                "Summarize.",
                "The Skills were listed.",
            ),
            Duration::ZERO,
        ),
        (
            chat_reasoning_answer_events("reasoning_content", "Go on.", "Going on."),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths) = service_with("reasoning-chat-compaction", |_, config| {
        configure_chat_provider(config, &server.base_url, "deepseek-v4-pro");
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();
    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    let outcome = service
        .start_turn(session.id, "Go on.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Going on.");
    stop(&service);
    let requests = server.requests();
    assert_eq!(requests.len(), 4, "no request was refused");
    let compaction = body(&requests[2]);
    assert!(!compaction["tools"].as_array().unwrap().is_empty());
    assert_eq!(
        reasoning_sent(&compaction),
        [
            Some("List first.".to_owned()),
            Some("Listed now.".to_owned())
        ]
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
