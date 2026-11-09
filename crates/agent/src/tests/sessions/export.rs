//! `export_session_markdown`: the session's committed conversation as
//! Markdown, read from its history.

use super::*;

// The managed turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_finished_turn_exports_as_markdown_and_an_empty_session_refuses() {
    let server = FakeServer::start(vec![(
        assistant_events("The answer is **42**."),
        Duration::ZERO,
    )])
    .await;
    let (service, paths) = configured_service("export-markdown", &server.base_url);
    service.start_plugins().await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();

    let empty = service.export_session_markdown(session.id).unwrap_err();
    assert!(
        matches!(empty, hanafubuki_agent_kernel::AgentError::InvalidInput(_)),
        "{empty:?}"
    );

    service
        .start_turn(session.id, "What is the answer?".to_string().into())
        .await
        .unwrap();
    let markdown = service.export_session_markdown(session.id).unwrap();

    assert_eq!(
        markdown,
        concat!(
            "# Hanafubuki Agent conversation\n",
            "\n## User\n\nWhat is the answer?\n",
            "\n## Assistant\n\nThe answer is **42**.\n",
        )
    );
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
