use super::*;

#[tokio::test]
async fn restart_repairs_a_turn_whose_items_were_all_compacted() {
    use hanafubuki_agent_kernel::{
        AgentConversationCommit, AgentHistoryMutation, AgentHistoryStore,
        EphemeralAgentHistoryStore,
    };

    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let model = ScriptedModel::new([
        tool_response_with_usage_and_commentary(12_000, Some("tool analysis ".repeat(1_000))),
        response("compact", "summary", 20),
        ScriptedResponse::WaitForCancellation,
    ]);
    let service = AgentService::with_history_store(
        Arc::new(model.clone()),
        FakeTools::new([FakeRegisteredExchange::success(
            "list_instances",
            json!({}),
            json!({"instances": []}),
        )])
        .await
        .registries(),
        store.clone(),
    );
    let mut settings = config();
    settings.context_window_tokens = Some(15_000);
    settings.auto_compact_threshold_percent = 67;
    let session = service.create_session(settings).unwrap();
    let running = tokio::spawn({
        let service = service.clone();
        async move { service.start_turn(session.id, "list ".repeat(1_000)).await }
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while model.requests().len() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let interrupted = store.load_runtime_state(session.id).unwrap();
    let (context, _) = service.test_history_backing(session.id).unwrap();
    assert!(context
        .iter()
        .all(|item| item.turn_id != interrupted.turn.as_ref().map(|turn| turn.id)));
    running.abort();
    let _ = running.await;
    // Use the actual checkpoint and model context from before the drop
    // guard, as a process crash would leave them for startup recovery.
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    store
        .create_session(AgentConversationCommit {
            runtime: interrupted,
            history_mutations: context
                .iter()
                .map(|item| AgentHistoryMutation::AppendItem {
                    item: item.as_ref().clone(),
                })
                .collect(),
        })
        .unwrap();
    let resumed_model = Arc::new(ScriptedModel::new([response(
        "resumed",
        "Recovered after compaction.",
        20,
    )]));
    let restarted = AgentService::with_history_store(
        resumed_model.clone(),
        FakeTools::new([]).await.registries(),
        store,
    );
    let report = restarted
        .recover_sessions_with_test_model(resumed_model)
        .unwrap();
    assert_eq!(report.restored, 1, "{report:?}");
    assert_eq!(report.interrupted_turns, vec![session.id]);
    assert_eq!(
        restarted
            .start_turn(session.id, "Continue.")
            .await
            .unwrap()
            .final_answer,
        "Recovered after compaction."
    );
}

#[test]
fn token_status_percentage_is_saturating_and_boundary_exact() {
    use hanafubuki_agent_kernel::AgentContextTokenStatus;
    // The share saturates, and the room for a reply still comes off it.
    let maximum = AgentContextTokenStatus::calculate(u64::MAX, Some(u64::MAX), None, true, 100);
    assert_eq!(
        maximum.proactive_compaction_token_limit,
        Some(u64::MAX - 32_768)
    );
    assert_eq!(maximum.base_window_tokens_remaining, Some(0));
    assert!(maximum.token_limit_reached);

    let boundary = AgentContextTokenStatus::calculate(80_000, Some(100_000), None, true, 80);
    assert_eq!(boundary.proactive_compaction_token_limit, Some(80_000));
    assert_eq!(boundary.base_window_tokens_remaining, Some(0));
    assert!(boundary.token_limit_reached);
    assert!(
        !AgentContextTokenStatus::calculate(79_999, Some(100_000), None, true, 80)
            .token_limit_reached
    );

    let disabled = AgentContextTokenStatus::calculate(100, Some(100), None, false, 80);
    assert_eq!(disabled.auto_compact_threshold_percent, None);
    assert_eq!(disabled.proactive_compaction_token_limit, None);
    assert!(!disabled.token_limit_reached);

    let unknown_window = AgentContextTokenStatus::calculate(100, None, None, true, 80);
    assert_eq!(unknown_window.proactive_compaction_token_limit, None);
    assert_eq!(unknown_window.base_window_tokens_remaining, None);
    assert!(!unknown_window.token_limit_reached);
}

/// The proactive limit is the threshold's share of the window, or less when
/// what is left could not hold a reply: the model's largest output (32768
/// when unknown), at most a tenth of the window but at least 4096 tokens,
/// and never more than half the window.
#[test]
fn the_proactive_limit_leaves_room_for_a_reply() {
    let limit = |window, max_output, percent| {
        hanafubuki_agent_kernel::AgentContextTokenStatus::calculate(
            0,
            Some(window),
            max_output,
            true,
            percent,
        )
        .proactive_compaction_token_limit
        .unwrap()
    };
    // Unknown largest output: a tenth of a large window, which the
    // default threshold already leaves.
    assert_eq!(limit(100_000, None, 80), 80_000);
    assert_eq!(limit(100_000, None, 95), 90_000);
    // A small window keeps 4096 tokens, or half of it.
    assert_eq!(limit(15_000, None, 80), 10_904);
    assert_eq!(limit(6_000, None, 80), 3_000);
    // A known largest output under a tenth of the window is the room kept.
    assert_eq!(limit(200_000, Some(8_192), 100), 191_808);
    assert_eq!(limit(20_000, Some(1_000), 100), 19_000);
    assert_eq!(limit(20_000, Some(1_000), 80), 16_000);
}

#[tokio::test]
async fn local_compaction_reads_canonical_plan_text_while_transcript_keeps_typed_projection() {
    let canonical = "<proposed_plan>\n# Plan\n\n- Preserve this.\n</proposed_plan>";
    let model = ScriptedModel::new([
        response("plan", canonical, 10),
        response("second", "Additional context", 10),
        response("compact", "Summary retaining the plan", 10),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut session_config = config();
    session_config.mode = hanafubuki_agent_kernel::AgentSessionMode::Plan;
    let session = service.create_session(session_config).unwrap();
    service.start_turn(session.id, "first").await.unwrap();
    service.start_turn(session.id, "second").await.unwrap();
    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();

    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].input.iter().any(|item| matches!(
        item,
        ModelInputItem::AssistantMessage { text, .. } if text == canonical
    )));
    let opened = service.open_session(session.id, 100).unwrap();
    assert!(opened.transcript.items.iter().any(|item| matches!(
        &item.content,
        ConversationItem::AssistantMessage {
            text,
            model_text: Some(model_text),
            proposed_plan: Some(plan),
            ..
        } if text.is_empty()
            && model_text == canonical
            && plan.text.contains("Preserve this")
    )));
}

#[tokio::test]
async fn fork_before_turn_works_on_both_sides_of_a_compaction_checkpoint() {
    let model = ScriptedModel::new([
        response("first", "first answer", 10),
        response("second", "second answer", 10),
        response("compact", "summary across the first two turns", 10),
        response("third", "third answer", 10),
        response("fork-first", "fork first answer", 10),
        response("fork-third", "fork third answer", 10),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut session_config = config();
    session_config.reasoning_effort = Some(hanafubuki_agent_model_api::ModelReasoningEffort::High);
    let session = service.create_session(session_config).unwrap();
    service
        .start_turn(session.id, "first prompt")
        .await
        .unwrap();
    let first_turn_id = service.snapshot(session.id).unwrap().turn.unwrap().id;
    service
        .start_turn(session.id, "second prompt")
        .await
        .unwrap();
    let before_compaction = service.snapshot(session.id).unwrap();
    service
        .compact_session(
            session.id,
            before_compaction.generation,
            before_compaction.revision,
        )
        .await
        .unwrap();
    service
        .start_turn(session.id, "third prompt")
        .await
        .unwrap();
    let completed = service.snapshot(session.id).unwrap();
    let third_turn_id = completed.turn.as_ref().unwrap().id;
    let (source_backing, source_history_version) =
        service.test_history_backing(session.id).unwrap();

    let head = service
        .fork_session_at_head(hanafubuki_agent_kernel::AgentSessionHeadForkRequest {
            source_session_id: session.id,
            source_generation: completed.generation,
            source_revision: completed.revision,
        })
        .unwrap();
    assert_eq!(head.source_revision, completed.revision);
    assert_eq!(head.session.forked_from_session_id, Some(session.id));
    assert_eq!(head.session.provider_id, completed.provider_id);
    assert_eq!(head.session.model, completed.model);
    assert_eq!(head.session.reasoning_effort, completed.reasoning_effort);
    assert_eq!(head.session.context_window, completed.context_window);
    assert!(head.session.usage.is_none());
    assert!(head.session.turn.is_none());
    assert!(head.session.compaction.is_none());
    assert!(head.session.active_activities.is_empty());
    assert_eq!(
        service.test_model_context(head.session.id).unwrap(),
        service.test_model_context(session.id).unwrap()
    );
    let (head_backing, head_history_version) =
        service.test_history_backing(head.session.id).unwrap();
    assert!(Arc::ptr_eq(&source_backing, &head_backing));
    assert_eq!(head_history_version, source_history_version);
    assert!(matches!(
        service.fork_session_at_head(hanafubuki_agent_kernel::AgentSessionHeadForkRequest {
            source_session_id: session.id,
            source_generation: completed.generation,
            source_revision: AgentRevision::new(completed.revision.value() - 1),
        }),
        Err(hanafubuki_agent_kernel::AgentError::StaleRevision)
    ));

    let before_first = service
        .fork_session_before_turn(hanafubuki_agent_kernel::AgentSessionForkRequest {
            source_session_id: session.id,
            source_generation: completed.generation,
            source_revision: completed.revision,
            before_turn_id: first_turn_id,
        })
        .unwrap();
    assert_eq!(before_first.restored_prompt, "first prompt");
    assert!(service
        .test_model_context(before_first.session.id)
        .unwrap()
        .is_empty());
    let fork_snapshot = service.snapshot(before_first.session.id).unwrap();
    service
        .set_session_mode(
            before_first.session.id,
            fork_snapshot.generation,
            fork_snapshot.revision,
            hanafubuki_agent_kernel::AgentSessionMode::Plan,
        )
        .unwrap();
    service
        .start_turn(before_first.session.id, "forked before first")
        .await
        .unwrap();

    let before_third = service
        .fork_session_before_turn(hanafubuki_agent_kernel::AgentSessionForkRequest {
            source_session_id: session.id,
            source_generation: completed.generation,
            source_revision: completed.revision,
            before_turn_id: third_turn_id,
        })
        .unwrap();
    assert_eq!(before_third.restored_prompt, "third prompt");
    let compacted_context = service.test_model_context(before_third.session.id).unwrap();
    assert!(matches!(
        compacted_context.last(),
        Some(ConversationItem::CompactionSummary { summary, .. })
            if summary.contains("summary across the first two turns")
    ));
    service
        .start_turn(before_third.session.id, "forked before third")
        .await
        .unwrap();

    let requests = model.requests();
    assert_eq!(requests.len(), 6);
    assert_eq!(
        requests
            .iter()
            .map(|request| request
                .reasoning
                .as_ref()
                .map(|reasoning| &reasoning.effort))
            .collect::<Vec<_>>(),
        vec![
            Some(&hanafubuki_agent_model_api::ModelReasoningEffort::High),
            Some(&hanafubuki_agent_model_api::ModelReasoningEffort::High),
            Some(&hanafubuki_agent_model_api::ModelReasoningEffort::High),
            Some(&hanafubuki_agent_model_api::ModelReasoningEffort::High),
            Some(&hanafubuki_agent_model_api::ModelReasoningEffort::High),
            Some(&hanafubuki_agent_model_api::ModelReasoningEffort::High),
        ]
    );
}

#[tokio::test]
async fn side_conversation_inherits_compacted_history_and_accepts_its_policy() {
    use hanafubuki_agent_kernel::{
        AgentChildSessionBase, AgentChildSessionRequest, AgentContinuationHookGeneration,
        AgentContinuationHookId, AgentContinuationHookProviderId, AgentSessionOrigin,
        AgentSessionPersistence,
    };
    let model = ScriptedModel::new([
        response("parent", "parent answer", 10),
        response("compact", "compressed parent", 10),
        response("child", "side answer", 10),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let parent = service.create_session(config()).unwrap();
    service
        .start_turn(parent.id, "parent question")
        .await
        .unwrap();
    let before = service.snapshot(parent.id).unwrap();
    service
        .compact_session(parent.id, before.generation, before.revision)
        .await
        .unwrap();
    let policy = ConversationItem::ContinuationContext {
        provider_id: AgentContinuationHookProviderId::parse("test".into()).unwrap(),
        hook_id: AgentContinuationHookId::parse("side".into()).unwrap(),
        generation: AgentContinuationHookGeneration([0; 32]),
        content: "Answer the side question only.".into(),
    };
    let child = service
        .create_child_session(AgentChildSessionRequest {
            parent_session_id: parent.id,
            origin: AgentSessionOrigin::Side,
            persistence: AgentSessionPersistence::Ephemeral,
            base: AgentChildSessionBase::ParentHead,
            context: vec![policy.clone()],
            scope: Default::default(),
            role_instructions: None,
        })
        .unwrap();
    assert_eq!(
        service.test_model_context(child.id).unwrap().last(),
        Some(&policy)
    );
    assert_eq!(
        service
            .start_turn(child.id, "side question")
            .await
            .unwrap()
            .final_answer,
        "side answer"
    );
    assert!(!service
        .test_model_context(parent.id)
        .unwrap()
        .contains(&policy));
}

#[tokio::test]
async fn a_fork_before_an_older_compacted_turn_ignores_transcript_only_summaries() {
    let model = ScriptedModel::new([
        response("first", "first answer", 10),
        response("compact-one", "first summary", 10),
        response("second", "second answer", 10),
        response("compact-two", "second summary", 10),
    ]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(session.id, "first prompt")
        .await
        .unwrap();
    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    service
        .start_turn(session.id, "second prompt")
        .await
        .unwrap();
    let before = service.snapshot(session.id).unwrap();
    let turn = before.turn.unwrap().id;
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    let source = service.snapshot(session.id).unwrap();
    let fork = service
        .fork_session_before_turn(hanafubuki_agent_kernel::AgentSessionForkRequest {
            source_session_id: session.id,
            source_generation: source.generation,
            source_revision: source.revision,
            before_turn_id: turn,
        })
        .unwrap();
    assert_eq!(fork.restored_prompt, "second prompt");
    let context = service.test_model_context(fork.session.id).unwrap();
    assert!(context.iter().any(|item| matches!(item, ConversationItem::AssistantMessage {text, ..} if text == "first answer")));
    assert!(!context
        .iter()
        .any(|item| matches!(item, ConversationItem::CompactionSummary { .. })));
}
