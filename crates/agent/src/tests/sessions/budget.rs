use super::*;
use crate::test_support::{reopen_service, test_host, with_input_tokens};

#[tokio::test(flavor = "multi_thread")]
async fn resumed_sessions_take_current_budgets_and_preserve_explicit_limits() {
    let server = FakeServer::start(vec![
        (
            with_input_tokens(assistant_events("Continued."), 101),
            Duration::ZERO,
        ),
        (
            with_input_tokens(assistant_events("Over budget."), 101),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths, config) = service_with_config("resumed-turn-budget", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        config
            .mutate_providers(1, |registry| {
                registry.model_policy.turn_budget = Some(
                    serde_json::from_value(json!({
                        "maximumCumulativeModelInputTokens": 100,
                        "maximumPendingInputItems": 64,
                        "maximumInFlightTools": 8,
                        "modelStreamIdleTimeoutSeconds": 120,
                        "toolIdleTimeoutSeconds": 600,
                        "approvalTimeoutSeconds": 900
                    }))
                    .unwrap(),
                );
                Ok(())
            })
            .unwrap();
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    drop(service);

    let explicit = config.providers().unwrap().model_policy.turn_budget;
    config
        .mutate_providers(2, |registry| {
            registry.model_policy.turn_budget = None;
            Ok(())
        })
        .unwrap();
    let service = reopen_service(config.clone(), test_host(&paths), &paths);
    let outcome = service
        .start_turn(session.id, "Continue".into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Continued.");

    config
        .mutate_providers(3, |registry| {
            registry.model_policy.turn_budget = explicit;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        service.start_turn(session.id, "Again".into()).await,
        Err(AgentError::LimitExceeded {
            resource: "cumulative model input tokens",
            limit: 100
        })
    ));
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
