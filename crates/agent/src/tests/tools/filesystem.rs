use super::*;
use serde_json::Value;

#[tokio::test]
async fn direct_file_read_is_unreviewed_and_returns_bounded_content() {
    let file = std::env::current_dir().unwrap().join("Cargo.toml");
    let arguments = json!({
        "path": file.to_string_lossy(),
        "offset": 0,
        "maxBytes": 8
    })
    .to_string();
    let server = FakeServer::start(vec![
        (tool_events("read_file", &arguments), Duration::ZERO),
        (assistant_events("The file was read."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("approved-file-read", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert_eq!(
        service
            .start_turn(session.id, "Read the exact file.".to_string().into())
            .await
            .unwrap()
            .final_answer,
        "The file was read."
    );
    assert!(!service
        .events_since(session.id, None)
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. })));
    let second: Value = serde_json::from_slice(&server.requests()[1].body).unwrap();
    let output = second["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .unwrap()
        .to_string();
    assert!(
        output.contains("[package"),
        "unexpected file output: {output}"
    );
    assert!(output.contains("truncated"));
    assert!(!output.contains("bounded content"));
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
