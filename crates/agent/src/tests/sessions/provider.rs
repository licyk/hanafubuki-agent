use super::*;

#[tokio::test]
async fn provider_write_commits_the_complete_provider_in_one_call() {
    let (service, paths) = service_with("whole-provider-write", |_, _| {});
    let mut draft = provider_draft("https://api.example.test/v1");
    draft.display_name = "Whole Provider".into();
    draft.api_token = Some("sk-initial".into());
    draft.request_headers = vec![AgentProviderHeaderSettings {
        name: "X-Organization".into(),
        value: "acme".into(),
    }];

    let created = service
        .write_provider(AgentProviderWriteRequest {
            expected_revision: 0,
            provider_id: None,
            provider: draft.clone(),
        })
        .await
        .unwrap();

    let provider = &created.providers[0];
    assert_eq!(provider.display_name, "Whole Provider");
    assert_eq!(provider.api_token.as_deref(), Some("sk-initial"));
    assert_eq!(provider.request_headers[0].value, "acme");
    // An eligible first Provider becomes the default without the caller
    // computing one.
    assert_eq!(
        created.default_provider_id.as_deref(),
        Some(provider.id.as_str())
    );

    let provider_id = provider.id.clone();
    draft.display_name = "Renamed Provider".into();
    draft.api_token = Some("sk-rotated".into());
    draft.request_headers.clear();
    let updated = service
        .write_provider(AgentProviderWriteRequest {
            expected_revision: created.revision,
            provider_id: Some(provider_id.clone()),
            provider: draft.clone(),
        })
        .await
        .unwrap();

    assert_eq!(updated.providers[0].display_name, "Renamed Provider");
    assert_eq!(
        updated.providers[0].api_token.as_deref(),
        Some("sk-rotated")
    );
    assert!(updated.providers[0].request_headers.is_empty());

    // Disabling the default Provider resolves a new default in Rust; the
    // registry is never left pointing at something unusable.
    draft.enabled = false;
    let disabled = service
        .write_provider(AgentProviderWriteRequest {
            expected_revision: updated.revision,
            provider_id: Some(provider_id),
            provider: draft,
        })
        .await
        .unwrap();
    assert_eq!(disabled.default_provider_id, None);

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The model policy is registry state, not Provider state: writing one must
/// not disturb the other, and the policy applies to whichever Provider runs.
#[tokio::test]
async fn model_policy_is_shared_and_independent_of_any_provider() {
    let (service, paths) = service_with("shared-model-policy", |_, config| {
        configure_test_provider(config, "https://api.example.test/v1", None);
    });
    let before = service.provider_registry_snapshot().await.unwrap();
    let mut policy = before.model_policy.clone();
    policy.context_window_tokens = Some(64_000);
    policy.reasoning_effort = Some(hanafubuki_agent_kernel::ModelReasoningEffort::High);

    let written = service
        .write_model_policy(AgentModelPolicyWriteRequest {
            expected_revision: before.revision,
            default_provider_id: before.default_provider_id.clone(),
            policy: policy.clone(),
        })
        .await
        .unwrap();

    assert_eq!(written.model_policy, policy);
    // Providers are untouched by a policy write.
    assert_eq!(written.providers, before.providers);

    // A Provider write leaves the policy alone in turn.
    let after = service
        .write_provider(AgentProviderWriteRequest {
            expected_revision: written.revision,
            provider_id: Some("provider-1".into()),
            provider: provider_draft("https://renamed.example.test/v1"),
        })
        .await
        .unwrap();
    assert_eq!(after.model_policy, policy);
    assert_eq!(
        after.providers[0].base_url,
        "https://renamed.example.test/v1"
    );

    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The model is registry state paired with the Provider that serves it, so one
/// write settles both and a Provider record never carries a model.
#[tokio::test]
async fn the_model_and_its_provider_are_chosen_in_one_write() {
    let (service, paths) = service_with("model-selection", |_, config| {
        configure_test_provider(config, "https://api.example.test/v1", None);
    });
    let current = service.provider_registry_snapshot().await.unwrap();
    let secondary = service
        .write_provider(AgentProviderWriteRequest {
            expected_revision: current.revision,
            provider_id: None,
            provider: provider_draft("https://secondary.example.test/v1"),
        })
        .await
        .unwrap();
    let secondary_id = secondary
        .providers
        .iter()
        .find(|provider| provider.id != "provider-1")
        .map(|provider| provider.id.clone())
        .expect("the second Provider exists");

    let mut policy = secondary.model_policy.clone();
    policy.model = "secondary-model".into();
    let written = service
        .write_model_policy(AgentModelPolicyWriteRequest {
            expected_revision: secondary.revision,
            default_provider_id: Some(secondary_id.clone()),
            policy,
        })
        .await
        .unwrap();

    assert_eq!(written.model_policy.model, "secondary-model");
    assert_eq!(
        written.default_provider_id.as_deref(),
        Some(secondary_id.as_str())
    );

    // A disabled Provider cannot be the one serving the model.
    let disabled = service
        .set_provider_enabled(AgentProviderEnabledRequest {
            expected_revision: written.revision,
            provider_id: secondary_id.clone(),
            enabled: false,
        })
        .await
        .unwrap();
    assert_ne!(
        disabled.default_provider_id.as_deref(),
        Some(secondary_id.as_str())
    );
    // The chosen model survives the Provider becoming unavailable.
    assert_eq!(disabled.model_policy.model, "secondary-model");

    let error = service
        .write_model_policy(AgentModelPolicyWriteRequest {
            expected_revision: disabled.revision,
            default_provider_id: Some(secondary_id),
            policy: disabled.model_policy.clone(),
        })
        .await
        .unwrap_err();
    assert!(error.contains("disabled"), "unexpected error: {error}");

    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A shared policy must survive a Provider that cannot honour part of it, so
/// values only Responses understands are narrowed at resolution rather than
/// rejected at write time.
#[tokio::test]
async fn responses_only_policy_values_survive_a_protocol_switch() {
    let (service, paths) = service_with("policy-narrowing", |_, config| {
        configure_test_provider(config, "https://api.example.test/v1", None);
    });
    let current = service.provider_registry_snapshot().await.unwrap();
    let mut policy = current.model_policy.clone();
    policy.reasoning_effort = Some(hanafubuki_agent_kernel::ModelReasoningEffort::High);
    policy.remote_compaction =
        hanafubuki_agent_config::AgentRemoteCompactionCapability::ResponsesCompact;
    let written = service
        .write_model_policy(AgentModelPolicyWriteRequest {
            expected_revision: current.revision,
            default_provider_id: current.default_provider_id.clone(),
            policy,
        })
        .await
        .unwrap();

    let mut anthropic = provider_draft("https://api.anthropic.test/v1");
    anthropic.protocol = AgentModelProtocol::AnthropicMessages;
    let switched = service
        .write_provider(AgentProviderWriteRequest {
            expected_revision: written.revision,
            provider_id: Some("provider-1".into()),
            provider: anthropic,
        })
        .await
        .unwrap();

    // The configured policy is kept verbatim rather than cleared or rejected;
    // a session on the Anthropic Provider is narrowed by the kernel (its
    // session config normalization), so switching back restores the full
    // behaviour.
    assert_eq!(
        switched.model_policy.reasoning_effort,
        Some(hanafubuki_agent_kernel::ModelReasoningEffort::High)
    );
    assert_eq!(
        switched.model_policy.remote_compaction,
        hanafubuki_agent_config::AgentRemoteCompactionCapability::ResponsesCompact
    );
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    assert_eq!(
        session.provider_protocol,
        hanafubuki_agent_kernel::ModelApiProtocol::AnthropicMessages
    );
    assert!(
        !session.model.is_empty(),
        "a configured session, not a neutral one"
    );
    assert_eq!(session.reasoning_effort, None);

    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn a_rejected_provider_write_leaves_the_stored_provider_untouched() {
    let (service, paths) = service_with("rejected-provider-write", |_, config| {
        configure_test_provider(config, "https://api.example.test/v1", None);
    });
    let before = service.provider_registry_snapshot().await.unwrap();
    let mut draft = provider_draft("https://api.example.test/v1");
    draft.api_token = Some("sk-valid".into());
    draft.base_url = "not-a-url".into();

    assert!(service
        .write_provider(AgentProviderWriteRequest {
            expected_revision: before.revision,
            provider_id: Some("provider-1".into()),
            provider: draft,
        })
        .await
        .is_err());

    // No partial commit: neither the endpoint nor the credential moved.
    let after = service.provider_registry_snapshot().await.unwrap();
    assert_eq!(after, before);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn create_session_uses_the_api_token_stored_with_the_provider() {
    let (service, paths) = service_with("direct-api-token", |_, config| {
        configure_test_provider(config, "https://api.example.test/v1", None);
    });
    write_fixture_credentials(
        &service,
        "https://api.example.test/v1",
        "visible-api-token",
        vec![],
    )
    .await;
    assert!(service
        .create_session(AgentSessionMode::Default)
        .await
        .is_ok());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn deleting_a_provider_removes_it_and_its_stored_credentials() {
    let (service, paths) = service_with("provider-delete", |_, config| {
        configure_test_provider(config, "https://api.example.test/v1", None);
    });
    let configured = write_fixture_credentials(
        &service,
        "https://api.example.test/v1",
        "token-value",
        vec![("X-Organization", "header-value")],
    )
    .await;
    assert_eq!(
        configured.providers[0].api_token.as_deref(),
        Some("token-value")
    );

    let deleted = service
        .delete_provider(AgentProviderDeleteRequest {
            expected_revision: configured.revision,
            provider_id: "provider-1".into(),
        })
        .await
        .unwrap();

    // The credentials lived in the Provider record, so removing it removes them.
    assert!(deleted.providers.is_empty());
    assert_eq!(deleted.default_provider_id, None);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[test]
fn model_catalog_request_contains_only_provider_ids() {
    let request = AgentProviderModelCatalogRequest {
        provider_ids: Some(vec!["provider-1".into()]),
    };
    let debug = format!("{request:?}");
    assert!(debug.contains("provider-1"));
    assert!(!debug.contains("base_url"));
    assert!(!debug.contains("api_token"));
}

#[tokio::test]
async fn model_catalog_sends_the_credentials_stored_with_the_provider() {
    let server = FakeServer::start(vec![(
        br#"{"data":[{"id":"model-a","display_name":"Model A"}]}"#.to_vec(),
        Duration::ZERO,
    )])
    .await;
    let (service, paths) = service_with("provider-model-catalog", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    write_fixture_credentials(
        &service,
        &server.base_url,
        "catalog-token",
        vec![("X-Provider-Scope", "catalog-header")],
    )
    .await;

    let groups = service
        .list_provider_models(AgentProviderModelCatalogRequest {
            provider_ids: Some(vec!["provider-1".into()]),
        })
        .await
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert!(
        groups[0].error.is_none(),
        "provider catalog failed: {:?}; captured requests: {}",
        groups[0].error,
        server.requests().len()
    );
    assert_eq!(
        groups[0]
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec!["model-a"]
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer catalog-token"
    );
    assert_eq!(
        requests[0].headers.get("x-provider-scope").unwrap(),
        "catalog-header"
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A draft's catalog is read as a saved Provider's would be, with its token
/// and headers, and the draft is not saved; a draft whose catalog cannot be
/// read gets why.
#[tokio::test]
async fn a_provider_draft_lists_its_models_without_being_saved() {
    let server = FakeServer::start(vec![(
        br#"{"data":[{"id":"model-a"}]}"#.to_vec(),
        Duration::ZERO,
    )])
    .await;
    let (service, paths) = service_with("provider-draft-catalog", |_, _| {});
    let mut draft = provider_draft(&server.base_url);
    draft.enabled = false;
    draft.api_token = Some("draft-token".into());
    draft.request_headers = vec![AgentProviderHeaderSettings {
        name: "X-Draft".into(),
        value: "draft-header".into(),
    }];

    let models = service.test_provider_draft(draft.clone()).await.unwrap();

    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["model-a"]
    );
    let requests = server.requests();
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer draft-token"
    );
    assert_eq!(requests[0].headers.get("x-draft").unwrap(), "draft-header");
    let registry = service.provider_registry_snapshot().await.unwrap();
    assert!(registry.providers.is_empty());
    assert_eq!(registry.revision, 0);

    draft.base_url = "not an endpoint".into();
    assert!(service.test_provider_draft(draft).await.is_err());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn model_catalog_keeps_successful_provider_results_when_another_provider_fails() {
    let server = FakeServer::start(vec![(
        br#"{"data":[{"id":"model-a"}]}"#.to_vec(),
        Duration::ZERO,
    )])
    .await;
    let unavailable_server = FakeServer::start(vec![(
        br#"{"error":{"message":"catalog unavailable"}}"#.to_vec(),
        Duration::ZERO,
    )])
    .await;
    let (service, paths, config) =
        service_with_config("provider-model-catalog-partial", |_, config| {
            configure_test_provider(config, &server.base_url, None);
        });
    config
        .mutate_providers(1, |registry| {
            let mut unavailable = registry.providers[0].clone();
            unavailable.id = "provider-2".into();
            unavailable.display_name = "Unavailable".into();
            unavailable.base_url = unavailable_server.base_url.clone();
            registry.providers.push(unavailable);
            Ok(())
        })
        .unwrap();

    let groups = service
        .list_provider_models(AgentProviderModelCatalogRequest { provider_ids: None })
        .await
        .unwrap();
    let available = groups
        .iter()
        .find(|group| group.provider_id == "provider-1")
        .unwrap();
    let unavailable = groups
        .iter()
        .find(|group| group.provider_id == "provider-2")
        .unwrap();
    assert!(
        available.error.is_none(),
        "available provider failed: {:?}",
        available.error
    );
    assert_eq!(
        available
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec!["model-a"]
    );
    assert!(unavailable.models.is_empty());
    assert!(unavailable.error.is_some());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn model_errors_and_snapshots_redact_the_configured_api_token() {
    let secret = "api-token-visible-locally";
    let header_secret = "header-value-visible-locally";
    let failure = format!(
        "data: {{\"type\":\"error\",\"error\":{{\"message\":\"rejected {secret} {header_secret}\"}}}}\n\n"
    )
            .into_bytes();
    let server = FakeServer::start(vec![(failure, Duration::ZERO)]).await;
    let (service, paths) = configured_service("diagnostic-content", &server.base_url);
    write_fixture_credentials(
        &service,
        &server.base_url,
        secret,
        vec![("X-Provider-Scope", header_secret)],
    )
    .await;
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let error = service
        .start_turn(session.id, "hello".into())
        .await
        .unwrap_err();
    let snapshot = service.snapshot(session.id).unwrap();

    assert!(!error.to_string().contains(secret));
    assert!(!error.to_string().contains(header_secret));
    assert!(!format!("{snapshot:?}").contains(secret));
    assert!(!format!("{snapshot:?}").contains(header_secret));
    assert!(snapshot
        .turn
        .unwrap()
        .last_error
        .is_some_and(|error| error.contains("[redacted]")
            && !error.contains(secret)
            && !error.contains(header_secret)));
    let opened = service.open_session(session.id, 100).unwrap();
    let conversation = serde_json::to_vec(&opened).unwrap();
    assert!(!conversation
        .windows(secret.len())
        .any(|bytes| bytes == secret.as_bytes()));
    assert!(!conversation
        .windows(header_secret.len())
        .any(|bytes| bytes == header_secret.as_bytes()));
    let requests = server.requests();
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        &format!("Bearer {secret}")
    );
    assert_eq!(
        requests[0].headers.get("x-provider-scope").unwrap(),
        header_secret
    );
    assert!(service
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
    assert_history_tree_excludes(
        &paths.data_dir.join("agent-test/history"),
        &[secret.as_bytes(), header_secret.as_bytes()],
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

fn assert_history_tree_excludes(root: &std::path::Path, forbidden: &[&[u8]]) {
    for entry in std::fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            assert_history_tree_excludes(&path, forbidden);
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        for value in forbidden {
            assert!(
                !bytes.windows(value.len()).any(|window| window == *value),
                "sensitive value leaked into {}",
                path.display()
            );
        }
    }
}
