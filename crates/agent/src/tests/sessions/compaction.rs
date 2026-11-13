//! `[compaction]` and the model policy's compaction keys reaching sessions
//! that already exist.

use super::*;
use hanafubuki_agent_config::{AgentCompactionSettings, AgentRemoteCompactionCapability};
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentCompactionImplementation,
    AgentCompactionStatus, AgentCompactionStrategy, AgentSessionOrigin, AgentSessionPersistence,
};
use serde_json::Value;

use super::children::finish;

struct Discard;

impl AgentEventSink for Discard {
    fn emit(&self, _event: AgentEvent) {}
}

const REMOTE_COMPACTION: &str = r#"{"output":[{"type":"message","role":"user","content":[{"type":"input_text","text":"Hello."}]},{"type":"compaction","encrypted_content":"opaque-v1"}]}"#;

/// Changing `remoteCompaction` reaches a session the next time it is
/// activated, so its next compaction uses it rather than failing until a
/// model is selected again; and the revision a caller read before the call
/// is still current.
// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_changed_remote_compaction_reaches_a_session_before_it_compacts() {
    let server = FakeServer::start(vec![
        (assistant_events("Hi."), Duration::ZERO),
        (REMOTE_COMPACTION.as_bytes().to_vec(), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("remote-compaction-change", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "Hello.".to_string().into())
        .await
        .unwrap();

    let current = service.provider_registry_snapshot().await.unwrap();
    let mut policy = current.model_policy.clone();
    policy.remote_compaction = AgentRemoteCompactionCapability::ResponsesCompact;
    service
        .write_model_policy(AgentModelPolicyWriteRequest {
            expected_revision: current.revision,
            default_provider_id: current.default_provider_id.clone(),
            policy,
        })
        .await
        .unwrap();

    // As an interface does: read the session, then compact it.
    let read = service.snapshot(session.id).unwrap();
    let outcome = service
        .compact_session_with_sink(
            session.id,
            read.generation,
            read.revision,
            Arc::new(Discard),
        )
        .await
        .unwrap();
    assert_eq!(outcome.compaction.status, AgentCompactionStatus::Completed);
    assert_eq!(
        outcome.compaction.implementation,
        AgentCompactionImplementation::ResponsesCompact
    );
    let request: Value = serde_json::from_slice(&server.requests()[1].body).unwrap();
    assert!(
        request.get("stream").is_none(),
        "a compact request, not a sampling one: {request}"
    );
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_configured_strategy_reaches_sessions_between_turns() {
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO); 3]).await;
    let (service, paths) = configured_service("compaction-strategy", &server.base_url);
    let strategy = |id| service.core.session_compaction_strategy(id).unwrap();
    let write = |compaction: AgentCompactionSettings| {
        service.write_settings(AgentSettingsPatch {
            compaction: Some(compaction),
            ..AgentSettingsPatch::default()
        })
    };
    let first = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert_eq!(strategy(first.id), AgentCompactionStrategy::Standard);

    let mut compaction = AgentCompactionSettings {
        strategy: hanafubuki_agent_config::AgentCompactionStrategy::Hierarchical,
        ..AgentCompactionSettings::default()
    };
    write(compaction.clone()).await.unwrap();
    // A new session takes the setting; an existing one at its next turn.
    let second = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert_eq!(strategy(second.id), AgentCompactionStrategy::Hierarchical);
    assert_eq!(strategy(first.id), AgentCompactionStrategy::Standard);
    service
        .start_turn(first.id, "Go on.".to_string().into())
        .await
        .unwrap();
    assert_eq!(strategy(first.id), AgentCompactionStrategy::Hierarchical);

    // Background compaction that leaves children out: a child runs
    // standard, its parent background; an ephemeral child always runs
    // standard.
    compaction.strategy = hanafubuki_agent_config::AgentCompactionStrategy::Background;
    compaction.background.child_sessions = false;
    write(compaction.clone()).await.unwrap();
    service
        .start_turn(first.id, "Once more.".to_string().into())
        .await
        .unwrap();
    assert_eq!(strategy(first.id), AgentCompactionStrategy::Background);
    let driver = service.test_session_driver().clone();
    let child = |persistence, base| {
        driver
            .create_child(AgentChildSessionRequest {
                parent_session_id: first.id,
                origin: AgentSessionOrigin::Subagent,
                persistence,
                base,
                context: Vec::new(),
                scope: Default::default(),
                role_instructions: None,
            })
            .unwrap()
            .id
    };
    let durable = child(
        AgentSessionPersistence::Durable,
        AgentChildSessionBase::Empty,
    );
    let ephemeral = child(
        AgentSessionPersistence::Ephemeral,
        AgentChildSessionBase::Empty,
    );
    assert_eq!(strategy(durable), AgentCompactionStrategy::Standard);
    assert_eq!(strategy(ephemeral), AgentCompactionStrategy::Standard);
    driver.close(ephemeral).await.unwrap();

    // A fork of the parent's history takes the setting over the fork record.
    compaction.background.child_sessions = true;
    write(compaction).await.unwrap();
    let fork = child(
        AgentSessionPersistence::Durable,
        AgentChildSessionBase::ParentHead,
    );
    assert_eq!(strategy(fork), AgentCompactionStrategy::Background);
    finish(service, paths);
}

/// A strategy applied between operations is committed at the session's
/// revision, and a restart reads it back from the history.
#[tokio::test(flavor = "multi_thread")]
async fn a_strategy_applied_between_operations_survives_a_restart() {
    let (service, paths, config) =
        service_with_host("compaction-strategy-restart", test_host, |_, config| {
            configure_test_provider(config, "http://127.0.0.1:1/v1", None);
        });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert!(service
        .core
        .set_session_compaction_strategy(session.id, AgentCompactionStrategy::Hierarchical)
        .unwrap());
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let reopened = AgentApplicationService::new_with_host(
        config,
        test_host(&paths),
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        hanafubuki_agent_process::executor::ExecutorManager::default(),
    )
    .unwrap();
    reopened.core.activate_session(session.id).unwrap();
    assert_eq!(
        reopened
            .core
            .session_compaction_strategy(session.id)
            .unwrap(),
        AgentCompactionStrategy::Hierarchical
    );
    assert_eq!(
        reopened.snapshot(session.id).unwrap().revision,
        session.revision
    );
    finish(reopened, paths);
}

/// A local summary on Anthropic Messages, of a history that holds a tool
/// call: the request declares the turn's tools, which the summary may not
/// call (the fake answers tool blocks without tools with an error).
#[tokio::test(flavor = "multi_thread")]
async fn an_anthropic_local_summary_declares_the_turns_tools_and_calls_none() {
    let server = FakeServer::start(vec![
        (anthropic_tool_events("skills_list", "{}"), Duration::ZERO),
        (anthropic_assistant_events("Listed."), Duration::ZERO),
        (
            anthropic_assistant_events("The user listed the Skills."),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths) = service_with("anthropic-local-summary", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        config
            .mutate_providers(1, |registry| {
                registry.providers[0].protocol = AgentModelProtocol::AnthropicMessages;
                Ok(())
            })
            .unwrap();
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "List the Skills.".to_string().into())
        .await
        .unwrap();

    let read = service.snapshot(session.id).unwrap();
    let outcome = service
        .compact_session(session.id, read.generation, read.revision)
        .await
        .unwrap();
    assert_eq!(outcome.compaction.status, AgentCompactionStatus::Completed);
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    let body = |index: usize| -> Value { serde_json::from_slice(&requests[index].body).unwrap() };
    let (turn, summary) = (body(1), body(2));
    assert!(summary.to_string().contains("tool_result"));
    assert_eq!(summary["tools"], turn["tools"]);
    assert!(!summary["tools"].as_array().unwrap().is_empty());
    assert_eq!(summary["tool_choice"], serde_json::json!({"type": "none"}));
    finish(service, paths);
}

/// Anthropic counts its prompt cache apart from `input_tokens`; the context
/// consumed is the prompt total, so a request mostly read from the cache
/// still reaches the threshold (80% of 100,000) and the next turn compacts
/// first.
#[tokio::test(flavor = "multi_thread")]
async fn an_anthropic_cache_read_counts_toward_the_compaction_threshold() {
    // Long, so that the summary is smaller than what it replaces.
    let answer = "The answer is long. ".repeat(100);
    let cached_answer = String::from_utf8(anthropic_assistant_events(&answer))
        .unwrap()
        .replace(
            "\"input_tokens\":10,",
            "\"input_tokens\":1000,\"cache_read_input_tokens\":79000,\"cache_creation_input_tokens\":500,",
        )
        .into_bytes();
    let server = FakeServer::start(vec![
        (cached_answer, Duration::ZERO),
        (
            anthropic_assistant_events("The user asked to read."),
            Duration::ZERO,
        ),
        (anthropic_assistant_events("Still here."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = service_with("anthropic-cache-threshold", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        config
            .mutate_providers(1, |registry| {
                registry.providers[0].protocol = AgentModelProtocol::AnthropicMessages;
                registry.model_policy.context_window_tokens = Some(100_000);
                Ok(())
            })
            .unwrap();
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "Read it.".to_string().into())
        .await
        .unwrap();
    let usage = service.snapshot(session.id).unwrap().usage.unwrap();
    assert_eq!(usage.last_response_input_tokens, 80_500);
    assert_eq!(usage.context_consumed_tokens, Some(80_500));
    assert_eq!(usage.last_response_cache_read_tokens, Some(79_000));
    assert_eq!(usage.last_response_cache_write_tokens, Some(500));
    assert_eq!(usage.cache_reporting_responses, 1);

    service
        .start_turn(session.id, "Are you there?".to_string().into())
        .await
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    let summary: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert!(summary.to_string().contains("Read it."));
    assert_eq!(
        service
            .snapshot(session.id)
            .unwrap()
            .compaction
            .unwrap()
            .status,
        AgentCompactionStatus::Completed
    );
    finish(service, paths);
}
