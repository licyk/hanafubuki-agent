use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activating_a_restored_provider_keeps_the_reported_context_baseline() {
    let server = FakeServer::start(vec![
        (
            with_input_tokens(assistant_events("Recorded."), 525_000),
            Duration::ZERO,
        ),
        (
            with_input_tokens(assistant_events("Continued."), 525_100),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths, config) = service_with_config("context-usage-restart", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        config
            .mutate_providers(1, |registry| {
                registry.model_policy.context_window_tokens = Some(1_000_000);
                Ok(())
            })
            .unwrap();
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    Box::pin(service.start_turn(session.id, " configuration".repeat(400_000).into()))
        .await
        .unwrap();
    let before = service.core.snapshot(session.id).unwrap().usage.unwrap();
    assert!(before.context_estimated_tokens.unwrap() > 1_000_000);
    shutdown_service(service).await;

    let service = reopen_service(config, test_host(&paths), &paths);
    service.core.activate_session(session.id).unwrap();
    assert_eq!(
        service.core.snapshot(session.id).unwrap().usage.unwrap(),
        before
    );
    assert_eq!(
        Box::pin(service.start_turn(session.id, "Continue.".into()))
            .await
            .unwrap()
            .final_answer,
        "Continued."
    );
    assert!(service
        .core
        .snapshot(session.id)
        .unwrap()
        .compaction
        .is_none());
    assert_eq!(server.requests().len(), 2);
    shutdown_service(service).await;
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}
