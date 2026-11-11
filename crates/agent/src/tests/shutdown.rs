use super::*;
use hanafubuki_agent_kernel::AgentError;

#[tokio::test]
async fn shutdown_cancels_the_active_model_and_joins_the_tool_worker() {
    let server = FakeServer::start(vec![(Vec::new(), Duration::from_secs(30))]).await;
    let (service, paths) = configured_service("shutdown", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let executing = service.clone();
    let turn = tokio::spawn(async move { executing.start_turn(session.id, "wait".into()).await });
    for _ in 0..100 {
        if !server.requests().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let shutting_down = service.clone();
    let diagnostics = tokio::task::spawn_blocking(move || {
        shutting_down.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
    })
    .await
    .unwrap();

    assert!(diagnostics.is_empty());
    assert_eq!(turn.await.unwrap(), Err(AgentError::Cancelled));
    assert!(service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap_err()
        .contains("shutting down"));
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
