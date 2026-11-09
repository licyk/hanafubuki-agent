use super::*;

#[test]
fn opening_a_completed_session_reads_only_runtime_and_the_requested_transcript_page() {
    let store = Arc::new(RecordingHistoryStore::default());
    let creating_service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let created = creating_service
        .create_model_session(
            AgentSessionConfig {
                provider_id: "provider-1".to_string(),
                provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol::Responses,
                model: "fixture".to_string(),
                instructions: "test".to_string(),
                mode: hanafubuki_agent_kernel::AgentSessionMode::Default,
                reasoning_effort: None,
                context_window_tokens: None,
                input_capabilities: Default::default(),
                auto_compaction_enabled: true,
                auto_compact_threshold_percent: 80,
                compaction_capability: ModelCompactionCapability::LocalSummary,
                sampling_retry_policy: hanafubuki_agent_kernel::AgentSamplingRetryPolicy::default(),
            },
            hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
            ModelCompactionCapability::LocalSummary,
            hanafubuki_agent_kernel::AgentSessionWorkspace::default(),
        )
        .unwrap();
    drop(creating_service);

    let viewing_service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let opened = viewing_service.open_session(created.id, 25).unwrap();

    assert_eq!(opened.runtime.id, created.id);
    assert!(opened.transcript.items.is_empty());
    assert!(viewing_service
        .events_since(created.id, None)
        .unwrap()
        .events
        .is_empty());
    assert_eq!(store.load_model_context_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        viewing_service.snapshot(created.id),
        Err(hanafubuki_agent_kernel::AgentError::SessionNotFound)
    ));
}
