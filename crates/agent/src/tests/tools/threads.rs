use super::*;
use serde_json::Value;

#[tokio::test]
async fn referenced_task_is_read_live_through_the_model_tool_pipeline() {
    let server = FakeServer::start(vec![
        (
            reasoning_assistant_events("Earlier task answer."),
            Duration::ZERO,
        ),
        (
            tool_events("read_thread", r#"{"threadId":1}"#),
            Duration::ZERO,
        ),
        (
            assistant_events("I read the referenced conversation."),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths) = configured_service("read-referenced-thread", &server.base_url);
    let referenced = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert_eq!(referenced.id.value(), 1);
    service
        .start_turn(referenced.id, "Earlier task request.".into())
        .await
        .unwrap();
    let current = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let answer = service
        .start_turn(current.id, "Continue [@earlier task](thread://1)".into())
        .await
        .unwrap();
    assert_eq!(answer.final_answer, "I read the referenced conversation.");
    let request: Value = serde_json::from_slice(&server.requests()[2].body).unwrap();
    let output = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .unwrap()
        .to_string();
    assert!(output.contains("Earlier task request."), "{output}");
    assert!(output.contains("Earlier task answer."), "{output}");
    assert!(!output.contains("A summary that arrives whole."));
    assert!(!service
        .events_since(current.id, None)
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. })));
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
