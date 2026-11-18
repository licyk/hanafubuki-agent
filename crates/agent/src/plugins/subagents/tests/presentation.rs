//! The resolved spawn configuration and task survive history replay.

use super::*;
use crate::test_support::reopen_service;
use hanafubuki_agent_kernel::ModelReasoningEffort;

#[tokio::test(flavor = "multi_thread")]
async fn spawn_and_wait_presentations_survive_reopening_the_session() {
    let server = FakeServer::start(vec![
        (tool_calls_events(&[
            ("spawn", "spawn_agent", r#"{"task_name":"counter","message":"Count the files.\nReport the total.","fork_turns":"none"}"#),
            ("wait", "wait_agent", r#"{"timeout_ms":20000}"#),
        ]), Duration::ZERO),
        (assistant_events("Twelve files."), Duration::ZERO),
        (assistant_events("There are twelve."), Duration::ZERO),
    ]).await;
    let (service, paths, config) =
        service_with_host("subagent-display-replay", test_host, |_, config| {
            configure_test_provider(config, &server.base_url, Some(ModelReasoningEffort::High));
        });
    service.start_plugins().await;
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "Delegate the count.".to_string().into())
        .await
        .unwrap();
    let spawn = service.tool_display(session.id, "spawn").unwrap().unwrap();
    let wait = service.tool_display(session.id, "wait").unwrap().unwrap();
    let Some(crate::AgentToolDisplay::Text { text }) = &spawn.display else {
        panic!("spawn display")
    };
    assert!(text.contains("Agent: counter (agent "));
    assert!(text.contains("Model: fixture-model · Reasoning: high"));
    assert!(text.contains("Task: Count the files.\nReport the total."));
    let Some(crate::AgentToolDisplay::Text { text }) = &wait.display else {
        panic!("wait display")
    };
    assert!(text.contains("Completed\nTwelve files."));
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    drop(service);

    let reopened = reopen_service(config, test_host(&paths), &paths);
    reopened.start_plugins().await;
    assert_eq!(
        reopened.tool_display(session.id, "spawn").unwrap(),
        Some(spawn)
    );
    assert_eq!(
        reopened.tool_display(session.id, "wait").unwrap(),
        Some(wait)
    );
    finish(reopened, paths);
}
