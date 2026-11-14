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
    assert_presented(&service, session.id, &file.to_string_lossy());
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The call's header names its file from its start, and the finished call
/// is shown as the file's text, read back from history. A presenter reads
/// the arguments as the tool would; a tool without one shows its output.
fn assert_presented(
    service: &AgentApplicationService,
    session_id: hanafubuki_agent_kernel::AgentSessionId,
    path: &str,
) {
    let events = service.events_since(session_id, None).unwrap().events;
    let (call_id, subject) = events
        .iter()
        .find_map(|event| match &event.kind {
            AgentEventKind::ToolStarted {
                call_id,
                presentation,
                ..
            } => Some((call_id.clone(), presentation.subject.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(subject.as_deref(), Some(path));

    let shown = service.tool_display(session_id, &call_id).unwrap().unwrap();
    assert_eq!(shown.subject.as_deref(), Some(path));
    let Some(crate::AgentToolDisplay::FileRead {
        start_line,
        text,
        truncated,
        ..
    }) = shown.display
    else {
        panic!("a read shows its text: {:?}", shown.display);
    };
    assert_eq!((start_line, truncated), (Some(1), true));
    assert!(text.starts_with("[package"), "{text}");
    assert_eq!(
        service.tool_display(session_id, "no-such-call").unwrap(),
        None
    );

    // `session_id` is an integer in `write_stdin`'s schema.
    let poll = json!({ "session_id": "3", "chars": null });
    assert_eq!(
        service
            .present_tool_call(None, "write_stdin", &poll, None)
            .subject
            .as_deref(),
        Some("session 3")
    );
    let unknown = hanafubuki_agent_tool_contracts::ToolOutput::structured(
        json!({ "status": "ok", "items": [1, 2] }),
        false,
    );
    assert_eq!(
        service.present_tool_call(
            None,
            "not_a_tool",
            &poll,
            Some((&unknown, &Default::default()))
        ),
        crate::AgentToolCallDisplay {
            subject: None,
            display: Some(crate::AgentToolDisplay::Fields {
                rows: vec![
                    ("items".to_string(), "2 items".to_string()),
                    ("status".to_string(), "ok".to_string()),
                ],
            }),
        }
    );
}
