#![cfg(feature = "test-support")]

//! Capture actual Chat Completions requests from the public runtime. These
//! tests establish prefix stability, not a remote provider's cache policy.

use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::{
    chat_reasoning_answer_events, configure_test_provider, enable_two_skills, reopen_service,
    service_with_host, test_host, FakeServer, TEST_SHUTDOWN_DEADLINE,
};
use hanafubuki_agent::{AgentModelProtocol, AgentSessionMode};
use serde_json::{json, Value};

fn answer() -> Vec<u8> {
    chat_reasoning_answer_events("reasoning_content", "synthetic reasoning", "OK")
}

fn call(id: &str, name: &str, arguments: Value) -> Vec<u8> {
    let delta = json!({
        "id": id, "choices": [{"index": 0, "delta": {
            "role": "assistant", "reasoning_content": "synthetic reasoning",
            "tool_calls": [{"index": 0, "id": id, "type": "function",
                "function": {"name": name, "arguments": arguments.to_string()}}]
        }, "finish_reason": "tool_calls"}]
    });
    format!("data: {delta}\n\ndata: [DONE]\n\n").into_bytes()
}

fn requests(server: &FakeServer) -> Vec<Value> {
    server
        .requests()
        .iter()
        .map(|request| {
            assert!(request.path.ends_with("/chat/completions"));
            serde_json::from_slice(&request.body).unwrap()
        })
        .collect()
}

fn assert_prefix(previous: &Value, next: &Value) {
    assert!(previous["tools"] == next["tools"], "tool schemas changed");
    let previous = previous["messages"].as_array().unwrap();
    let next = next["messages"].as_array().unwrap();
    assert!(next.len() > previous.len(), "conversation did not grow");
    for (index, message) in previous.iter().enumerate() {
        assert!(
            message == &next[index],
            "previous wire message {index} changed (role={}, previous_bytes={}, next_bytes={})",
            message["role"],
            message.to_string().len(),
            next[index].to_string().len()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn world_state_replacement_removal_and_skill_catalog_changes_append_to_wire() {
    let server = FakeServer::start((0..5).map(|_| (answer(), Duration::ZERO)).collect()).await;
    let (service, paths, config) = service_with_host("cache-world", test_host, |_, config| {
        configure_test_provider(config, &server.base_url, None);
        let revision = config.providers().unwrap().revision;
        config
            .mutate_providers(revision, |registry| {
                registry.providers[0].protocol = AgentModelProtocol::ChatCompletions;
                Ok(())
            })
            .unwrap();
    });
    let instructions = paths.managed_instances_dir.join("AGENTS.md");
    std::fs::write(&instructions, "SYNTHETIC_RULE_ALPHA").unwrap();
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "Round one".into())
        .await
        .unwrap();
    std::fs::write(&instructions, "SYNTHETIC_RULE_BETA").unwrap();
    enable_two_skills(&service, &paths);
    service
        .start_turn(session.id, "Round two".into())
        .await
        .unwrap();
    std::fs::remove_file(instructions).unwrap();
    for skill in service.list_skills().unwrap().entries {
        service.disable_skill(&skill.id).unwrap();
    }
    service
        .start_turn(session.id, "Round three".into())
        .await
        .unwrap();
    service
        .start_turn(session.id, "Round four".into())
        .await
        .unwrap();
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    drop(service);
    let service = reopen_service(config, test_host(&paths), &paths);
    service
        .start_turn(session.id, "After restart".into())
        .await
        .unwrap();
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let captured = requests(&server);
    assert_eq!(captured.len(), 5);
    assert!(captured[0].to_string().contains("SYNTHETIC_RULE_ALPHA"));
    assert!(captured[1].to_string().contains("SYNTHETIC_RULE_BETA"));
    assert!(captured[1].to_string().contains("Available skills"));
    assert_eq!(
        captured[4].to_string().matches("Available skills").count(),
        1,
        "an unchanged catalog must not be repeated on every sampling step"
    );
    for pair in captured.windows(2) {
        assert_prefix(&pair[0], &pair[1]);
    }
    drop(service);
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn compaction_starts_one_new_canonical_prefix_then_preserves_it() {
    let server = FakeServer::start((0..5).map(|_| (answer(), Duration::ZERO)).collect()).await;
    let (service, paths, _) = service_with_host("cache-compact", test_host, |_, config| {
        configure_test_provider(config, &server.base_url, None);
        let revision = config.providers().unwrap().revision;
        config
            .mutate_providers(revision, |registry| {
                registry.providers[0].protocol = AgentModelProtocol::ChatCompletions;
                Ok(())
            })
            .unwrap();
    });
    let instructions = paths.managed_instances_dir.join("AGENTS.md");
    std::fs::write(&instructions, "SYNTHETIC_OLD_RULE").unwrap();
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "First turn".into())
        .await
        .unwrap();
    std::fs::write(&instructions, "SYNTHETIC_NEW_RULE").unwrap();
    service
        .start_turn(session.id, "Second turn".into())
        .await
        .unwrap();
    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    service
        .start_turn(session.id, "First turn after compaction".into())
        .await
        .unwrap();
    service
        .start_turn(session.id, "Second turn after compaction".into())
        .await
        .unwrap();
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let captured = requests(&server);
    assert_eq!(captured.len(), 5);
    assert!(captured[3].to_string().contains("SYNTHETIC_NEW_RULE"));
    assert!(!captured[3].to_string().contains("SYNTHETIC_OLD_RULE"));
    assert_prefix(&captured[3], &captured[4]);
    drop(service);
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_mode_after_compaction_keeps_the_baseline_explicitly_historical() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "Synthetic read-only planning fixture").unwrap();
    let first = String::from_utf8(call("read", "read_file", json!({"path": file.path(), "offset": 0, "maxBytes": 100}))).unwrap()
        .replace("data: [DONE]", "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":95000,\"completion_tokens\":1}}\n\ndata: [DONE]");
    let mut responses = vec![(first.into_bytes(), Duration::ZERO)];
    responses.extend((0..4).map(|_| (answer(), Duration::ZERO)));
    let server = FakeServer::start(responses).await;
    let (service, paths, _) = service_with_host("cache-mode", test_host, |_, config| {
        configure_test_provider(config, &server.base_url, None);
        let revision = config.providers().unwrap().revision;
        config
            .mutate_providers(revision, |registry| {
                registry.providers[0].protocol = AgentModelProtocol::ChatCompletions;
                registry.model_policy.context_window_tokens = Some(100_000);
                Ok(())
            })
            .unwrap();
    });
    let session = service
        .create_session(AgentSessionMode::Plan, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "Plan the synthetic task".into())
        .await
        .unwrap();
    let before = service.snapshot(session.id).unwrap();
    service
        .set_session_mode(
            session.id,
            before.generation,
            before.revision,
            AgentSessionMode::Default,
        )
        .unwrap();
    service
        .start_turn(session.id, "Use Default mode now".into())
        .await
        .unwrap();
    service
        .start_turn(session.id, "Continue in Default mode".into())
        .await
        .unwrap();
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );

    let captured = requests(&server);
    assert_eq!(captured.len(), 5);
    let canonical = |request: &Value| {
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| {
                message["content"]
                    .as_str()
                    .is_some_and(|text| text.contains("Window-start mode: plan"))
            })
            .cloned()
            .expect("the compacted window retains its original Plan baseline")
    };
    let plan_baseline = canonical(&captured[2]);
    assert_eq!(plan_baseline, canonical(&captured[3]));
    let text = plan_baseline["content"].as_str().unwrap();
    assert!(text.contains("Historical Agent baseline"));
    assert!(text.contains("not the current configuration or task state"));
    assert!(text.contains("current system instructions and tools determine the active mode"));
    assert!(captured[2]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("# Plan Mode (Conversational)"));
    assert!(!captured[3]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("# Plan Mode (Conversational)"));
    let advertises_patch = |request: &Value| {
        request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["function"]["name"] == "apply_patch")
    };
    assert!(!advertises_patch(&captured[2]));
    assert!(advertises_patch(&captured[3]));
    assert_prefix(&captured[3], &captured[4]);
    drop(service);
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn task_plan_and_growing_large_tool_history_keep_previous_wire_messages() {
    assert_tool_history(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn growing_large_tool_history_keeps_previous_outputs_without_task_plan() {
    assert_tool_history(false).await;
}

async fn assert_tool_history(with_plan: bool) {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "SYNTHETIC_OUTPUT_".repeat(16_000)).unwrap();
    let mut responses = Vec::new();
    if with_plan {
        responses.push(call(
            "plan",
            "update_plan",
            json!({"plan": [
                {"step": "Read synthetic data", "status": "in_progress"}
            ]}),
        ));
    }
    for index in 0..6 {
        responses.push(call(
            &format!("read-{index}"),
            "read_file",
            json!({
                "path": file.path(), "offset": index * 100, "maxBytes": 200_000
            }),
        ));
    }
    responses.push(answer());
    responses.push(answer());
    let server = FakeServer::start(
        responses
            .into_iter()
            .map(|body| (body, Duration::ZERO))
            .collect(),
    )
    .await;
    let (service, paths, _) = service_with_host("cache-tools", test_host, |_, config| {
        configure_test_provider(config, &server.base_url, None);
        let revision = config.providers().unwrap().revision;
        config
            .mutate_providers(revision, |registry| {
                registry.providers[0].protocol = AgentModelProtocol::ChatCompletions;
                registry.model_policy.context_window_tokens = Some(100_000);
                registry.model_policy.auto_compaction_enabled = false;
                Ok(())
            })
            .unwrap();
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "Read the synthetic fixture repeatedly".into())
        .await
        .unwrap();
    service
        .start_turn(session.id, "Continue".into())
        .await
        .unwrap();
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let captured = requests(&server);
    assert_eq!(captured.len(), 8 + usize::from(with_plan));
    assert!(
        captured[1 + usize::from(with_plan)]
            .to_string()
            .contains("SYNTHETIC_OUTPUT_"),
        "tool must actually read the fixture"
    );
    for pair in captured.windows(2) {
        assert_prefix(&pair[0], &pair[1]);
    }
    drop(service);
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}
