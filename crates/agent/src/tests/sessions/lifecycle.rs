use super::*;
use crate::AgentConfig;
use hanafubuki_agent_process::executor::ExecutorManager;
use serde_json::Value;

#[tokio::test]
async fn create_session_without_a_provider_reports_it_is_not_configured() {
    // The Agent UX keys its "configure in Settings" entry point off this exact
    // contract, so it is asserted here rather than left implicit.
    let (service, paths) = service_with("no-provider", |_, _| {});
    let error = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap_err();
    assert!(
        error.contains("default Agent Provider is configured"),
        "unexpected error: {error}"
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn create_session_without_a_global_model_waits_for_conversation_selection() {
    let (service, paths) = service_with("no-global-model", |_, config| {
        configure_test_provider(config, "http://127.0.0.1:1/v1", None);
        config
            .mutate_providers(1, |registry| {
                registry.model_policy.model.clear();
                Ok(())
            })
            .unwrap();
    });

    let created = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let opened = service.open_session(created.id, 75).unwrap().runtime;
    assert!(opened.model.is_empty());
    assert_eq!(opened.provider_id, "provider-1");

    let error = service
        .start_turn(created.id, "must wait for a model".into())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("use /model"),
        "unexpected error: {error}"
    );
    assert!(service
        .open_session(created.id, 75)
        .unwrap()
        .transcript
        .items
        .is_empty());

    let selected = service
        .set_session_model(AgentSessionModelSelectionRequest {
            session_id: opened.id,
            generation: opened.generation,
            revision: opened.revision,
            provider_id: "provider-1".into(),
            model: "conversation-model".into(),
        })
        .await
        .unwrap();
    assert_eq!(selected.model, "conversation-model");
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn session_model_selection_rejects_an_empty_model() {
    let (service, paths) = configured_service("empty-model-selection", "http://127.0.0.1:1/v1");
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let error = service
        .set_session_model(AgentSessionModelSelectionRequest {
            session_id: session.id,
            generation: session.generation,
            revision: session.revision,
            provider_id: "provider-1".into(),
            model: "   ".into(),
        })
        .await
        .unwrap_err();
    assert!(
        error.contains("Agent model must contain"),
        "unexpected error: {error}"
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The facade is where user images enter: more than a message carries is
/// refused before any is read or staged, and nothing reaches the session.
#[tokio::test]
async fn a_turn_with_too_many_images_is_refused_before_staging() {
    struct Discard;
    impl AgentEventSink for Discard {
        fn emit(&self, _event: AgentEvent) {}
    }
    let (service, paths) = configured_service("too-many-images", "http://127.0.0.1:1/v1");
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let image = DesktopAgentImageInput {
        path: Some(paths.app_data_dir.join("missing.png").display().to_string()),
        attachment: None,
        display_name: "missing.png".into(),
        detail: Default::default(),
    };
    let input = AgentTurnRequest {
        text: "look".into(),
        images: vec![image; hanafubuki_agent_kernel::MAXIMUM_USER_IMAGES_PER_MESSAGE + 1],
        skills: Vec::new(),
        host: Default::default(),
    };
    let error = service
        .submit_turn_with_sink(session.id, input, Arc::new(Discard))
        .await
        .unwrap_err();
    assert_eq!(
        error,
        format!(
            "at most {} images may be submitted",
            hanafubuki_agent_kernel::MAXIMUM_USER_IMAGES_PER_MESSAGE
        )
    );
    assert!(service
        .open_session(session.id, 75)
        .unwrap()
        .transcript
        .items
        .is_empty());
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn lifecycle_mutation_lazily_opens_a_completed_session_after_restart() {
    let (service, paths) = configured_service("history-lazy-lifecycle", "http://127.0.0.1:1/v1");
    let created = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let summary = service
        .list_sessions_page(AgentSessionListRequest::default())
        .unwrap()
        .sessions
        .into_iter()
        .find(|session| session.session_id == created.id)
        .unwrap();
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    drop(service);

    let restarted = AgentApplicationService::new(
        &paths,
        AgentConfig::new(&paths.config_dir),
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
    )
    .unwrap();
    let archived = restarted
        .archive_session(summary.session_id, summary.generation, summary.revision)
        .unwrap();
    assert_eq!(archived.lifecycle, AgentSessionLifecycle::Archived);
    assert!(restarted
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn model_selection_lazily_activates_an_opened_session_after_restart() {
    let (service, paths, config) =
        service_with_config("history-lazy-model-selection", |_, config| {
            configure_test_provider(config, "http://127.0.0.1:1/v1", None);
        });
    let created = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    drop(service);

    let restarted = AgentApplicationService::new(
        &paths,
        config,
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
    )
    .unwrap();
    let opened = restarted.open_session(created.id, 75).unwrap().runtime;
    let switched = restarted
        .set_session_model(AgentSessionModelSelectionRequest {
            session_id: opened.id,
            generation: opened.generation,
            revision: opened.revision,
            provider_id: "provider-1".into(),
            model: "replacement-model".into(),
        })
        .await
        .unwrap();
    assert_eq!(switched.model, "replacement-model");
    assert_eq!(switched.revision.value(), opened.revision.value() + 1);
    assert!(restarted
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn existing_sessions_keep_frozen_reasoning_when_global_settings_change() {
    let server = FakeServer::start(vec![
        (assistant_events("first"), Duration::ZERO),
        (assistant_events("second"), Duration::ZERO),
    ])
    .await;
    let (service, paths, config_state) = service_with_config("reasoning-freeze", |_, config| {
        configure_test_provider(
            config,
            &server.base_url,
            Some(hanafubuki_agent_kernel::ModelReasoningEffort::High),
        );
    });
    let first = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    config_state
        .mutate_providers(1, |registry| {
            registry.model_policy.reasoning_effort =
                Some(hanafubuki_agent_kernel::ModelReasoningEffort::Low);
            Ok(())
        })
        .unwrap();
    let second = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();

    service.start_turn(first.id, "first".into()).await.unwrap();
    service
        .start_turn(second.id, "second".into())
        .await
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    let first_body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let second_body: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(
        first_body["reasoning"],
        json!({"effort": "high", "summary": "auto"})
    );
    assert_eq!(
        second_body["reasoning"],
        json!({"effort": "low", "summary": "auto"})
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
