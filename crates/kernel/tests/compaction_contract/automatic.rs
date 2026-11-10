use super::*;

#[tokio::test]
async fn compaction_receives_the_current_task_checklist_as_typed_context() {
    let model = ScriptedModel::new([
        response("baseline", "Baseline recorded", 8),
        task_plan_response(),
        response("answer", "Work completed", 10),
        response("compact", "The work and its progress were summarized.", 8),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(session.id, "Record the baseline")
        .await
        .unwrap();
    service.start_turn(session.id, "Do the work").await.unwrap();
    let before = service.snapshot(session.id).unwrap();
    assert!(before.task_plan.is_some());

    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();

    let requests = model.requests();
    let task_plan = requests[3].input.iter().find_map(|item| match item {
        ModelInputItem::ContextFragment { kind, content, .. } if kind == "task_plan" => {
            Some(content.as_str())
        }
        _ => None,
    });
    assert!(task_plan.is_some_and(|content| {
        content.contains("Compaction must retain this progress")
            && content.contains("- [in_progress] Finish the compacted task")
    }));
}

#[tokio::test]
async fn auto_compaction_runs_mid_turn_only_before_followup_sampling() {
    let model = ScriptedModel::new([
        tool_response_with_usage_and_commentary(
            12_000,
            Some("transient tool analysis ".repeat(1_000)),
        ),
        response("compact", "summary", 20),
        response("answer", "No instances are configured.", 200),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        json!({"instances": []}),
    )])
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let mut config = config();
    config.context_window_tokens = Some(15_000);
    config.auto_compact_threshold_percent = 67;
    let session = service.create_session(config).unwrap();

    assert_eq!(
        service
            .start_turn(session.id, "list ".repeat(1_000))
            .await
            .unwrap()
            .final_answer,
        "No instances are configured."
    );
    let snapshot = test_session_view(&service, session.id).unwrap();
    assert_eq!(
        snapshot
            .runtime
            .compaction
            .as_ref()
            .map(|value| value.phase),
        Some(hanafubuki_agent_kernel::AgentCompactionPhase::MidTurn)
    );
    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    // The turn's tools, which the summary may not call.
    assert_eq!(requests[1].tools, requests[0].tools);
    assert_eq!(
        requests[1].tool_choice,
        hanafubuki_agent_model_api::ModelToolChoice::None
    );
}
#[tokio::test]
async fn auto_compaction_runs_once_at_sampling_admission_and_uses_new_history() {
    let model = ScriptedModel::new([
        response("one", &"transient answer ".repeat(1_000), 12_000),
        response("compact", "The first exchange is summarized.", 100),
        response("two", "second answer", 200),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut config = config();
    config.context_window_tokens = Some(15_000);
    config.auto_compact_threshold_percent = 67;
    let session = service.create_session(config).unwrap();
    service
        .start_turn(session.id, "first".to_string())
        .await
        .unwrap();
    let outcome = service
        .start_turn(session.id, "second".to_string())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "second answer");

    let snapshot = test_session_view(&service, session.id).unwrap();
    let compaction = snapshot.runtime.compaction.as_ref().unwrap();
    assert_eq!(compaction.trigger, AgentCompactionTrigger::Automatic);
    assert_eq!(compaction.status, AgentCompactionStatus::Completed);
    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].tools, requests[0].tools);
    assert_eq!(
        requests[1].tool_choice,
        hanafubuki_agent_model_api::ModelToolChoice::None
    );
    let summary_index = requests[2]
        .input
        .iter()
        .position(|item| {
            matches!(
                item,
                hanafubuki_agent_model_api::ModelInputItem::CompactionSummary { .. }
            )
        })
        .unwrap();
    let second_user_index = requests[2]
        .input
        .iter()
        .rposition(|item| {
            matches!(item, ModelInputItem::UserMessage { content } if format!("{content:?}").contains("second"))
        })
        .unwrap();
    assert!(summary_index < second_user_index);
}

#[tokio::test]
async fn provider_context_error_compacts_and_retries_the_same_turn() {
    let model = ScriptedModel::new([
        response("one", &"old context ".repeat(1_000), 100),
        ScriptedResponse::Events(vec![Err(ModelError::ContextWindowExceeded {
            status: Some(400),
            code: Some("context_length_exceeded".to_string()),
            message: "maximum context length exceeded".to_string(),
            request_id: Some("request-context".to_string()),
        })]),
        response("compact", "first exchange summary", 50),
        response("two", "second answer", 100),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "first").await.unwrap();

    let outcome = service.start_turn(session.id, "second").await.unwrap();

    assert_eq!(outcome.final_answer, "second answer");
    let snapshot = test_session_view(&service, session.id).unwrap();
    let compaction = snapshot.runtime.compaction.as_ref().unwrap();
    assert_eq!(compaction.trigger, AgentCompactionTrigger::Automatic);
    assert_eq!(
        compaction.reason,
        hanafubuki_agent_kernel::AgentCompactionReason::ContextLimit
    );
    assert_eq!(
        compaction.phase,
        hanafubuki_agent_kernel::AgentCompactionPhase::PreTurn
    );
    assert_eq!(snapshot.context_window.window_number, 1);
    assert_eq!(model.requests().len(), 4);
}

/// One compaction answers a context-window error per step: a second error
/// before a response succeeds ends the turn with its own reason, instead of
/// compacting again until the step limit.
#[tokio::test]
async fn a_second_provider_context_error_after_compaction_fails_the_turn() {
    let context_error = || {
        ScriptedResponse::Events(vec![Err(ModelError::ContextWindowExceeded {
            status: Some(400),
            code: Some("context_length_exceeded".to_string()),
            message: "maximum context length exceeded".to_string(),
            request_id: None,
        })])
    };
    let model = ScriptedModel::new([
        response("one", &"old context ".repeat(1_000), 100),
        context_error(),
        response("compact", "first exchange summary", 50),
        context_error(),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "first").await.unwrap();

    let error = service.start_turn(session.id, "second").await.unwrap_err();

    assert_eq!(
        error,
        AgentError::ContextWindowExceededAfterCompaction {
            message: "maximum context length exceeded".to_string()
        }
    );
    assert_eq!(model.requests().len(), 4);
    let snapshot = service.snapshot(session.id).unwrap();
    assert_eq!(snapshot.context_window.window_number, 1);
    let turn = snapshot.turn.unwrap();
    assert_eq!(turn.phase, hanafubuki_agent_kernel::AgentTurnPhase::Failed);
    assert_eq!(
        turn.terminal_reason,
        Some(hanafubuki_agent_kernel::AgentTurnTerminalReason::ContextWindowExceeded)
    );
    assert!(turn
        .last_error
        .is_some_and(|message| message.contains("still too long")));
}

#[tokio::test]
async fn provider_context_error_is_not_recovered_when_auto_compaction_is_disabled() {
    let model = ScriptedModel::new([
        response("one", &"old context ".repeat(1_000), 100),
        ScriptedResponse::Events(vec![Err(ModelError::ContextWindowExceeded {
            status: Some(400),
            code: Some("context_length_exceeded".to_string()),
            message: "maximum context length exceeded".to_string(),
            request_id: None,
        })]),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut settings = config();
    settings.auto_compaction_enabled = false;
    let session = service.create_session(settings).unwrap();
    service.start_turn(session.id, "first").await.unwrap();

    assert!(matches!(
        service.start_turn(session.id, "second").await,
        Err(AgentError::ProviderContextWindowExceeded { .. })
    ));
    assert_eq!(model.requests().len(), 2);
    assert!(service.snapshot(session.id).unwrap().compaction.is_none());
}

#[tokio::test]
async fn provider_context_error_without_a_safe_prefix_preserves_the_original_failure() {
    let model = ScriptedModel::new([ScriptedResponse::Events(vec![Err(
        ModelError::ContextWindowExceeded {
            status: Some(400),
            code: Some("context_length_exceeded".to_string()),
            message: "maximum context length exceeded".to_string(),
            request_id: None,
        },
    )])]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();

    assert!(matches!(
        service.start_turn(session.id, "first").await,
        Err(AgentError::ProviderContextWindowExceeded { .. })
    ));
    assert_eq!(model.requests().len(), 1);
    assert!(service.snapshot(session.id).unwrap().compaction.is_none());
}

#[tokio::test]
async fn provider_context_error_after_response_start_without_output_compacts_and_retries() {
    let model = ScriptedModel::new([
        response("one", &"old context ".repeat(1_000), 100),
        ScriptedResponse::Events(vec![
            Ok(ModelEvent::ResponseStarted {
                response_id: "partial".to_string(),
                metadata: ModelResponseMetadata::default(),
            }),
            Err(ModelError::ContextWindowExceeded {
                status: Some(400),
                code: Some("context_length_exceeded".to_string()),
                message: "maximum context length exceeded".to_string(),
                request_id: None,
            }),
        ]),
        response("compact", "first exchange summary", 50),
        response("two", "second answer", 100),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "first").await.unwrap();

    let outcome = service.start_turn(session.id, "second").await.unwrap();

    assert_eq!(outcome.final_answer, "second answer");
    let snapshot = test_session_view(&service, session.id).unwrap();
    let compaction = snapshot.runtime.compaction.as_ref().unwrap();
    assert_eq!(compaction.trigger, AgentCompactionTrigger::Automatic);
    assert_eq!(compaction.status, AgentCompactionStatus::Completed);
    assert_eq!(snapshot.context_window.window_number, 1);
    assert_eq!(model.requests().len(), 4);
}

#[tokio::test]
async fn provider_context_error_after_output_item_is_never_replayed() {
    let model = ScriptedModel::new([ScriptedResponse::Events(vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "partial".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id: "partial-item".to_string(),
                phase: Some(ModelMessagePhase::Commentary),
                text: String::new(),
            },
        }),
        Err(ModelError::ContextWindowExceeded {
            status: Some(400),
            code: Some("context_length_exceeded".to_string()),
            message: "maximum context length exceeded".to_string(),
            request_id: None,
        }),
    ])]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();

    assert!(matches!(
        service.start_turn(session.id, "first").await,
        Err(AgentError::Model(_))
    ));
    assert_eq!(model.requests().len(), 1);
    assert!(service.snapshot(session.id).unwrap().compaction.is_none());
}

#[tokio::test]
async fn automatic_compaction_that_does_not_reduce_context_fails_without_opening_a_window() {
    let model = ScriptedModel::new([
        response("one", "short answer", 12_000),
        response("compact", &"larger summary ".repeat(4_000), 100),
    ]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let mut config = config();
    config.context_window_tokens = Some(100_000);
    config.auto_compact_threshold_percent = 10;
    let session = service.create_session(config).unwrap();
    service.start_turn(session.id, "first").await.unwrap();
    let before = test_session_view(&service, session.id).unwrap();

    assert!(matches!(
        service.start_turn(session.id, "second").await,
        Err(AgentError::CompactionDidNotReduceContext { .. })
    ));
    let after = test_session_view(&service, session.id).unwrap();
    assert_eq!(after.context_window.window_number, 0);
    assert_eq!(
        after.model_context[..before.model_context.len()],
        before.model_context
    );
    assert_eq!(
        after.runtime.compaction.as_ref().unwrap().status,
        AgentCompactionStatus::Failed
    );
}

#[tokio::test]
async fn budget_pressure_without_a_window_hint_compacts_mid_turn_and_restores_headroom() {
    let model = ScriptedModel::new([
        tool_response_with_usage_and_commentary(90, Some("transient tool analysis ".repeat(1_000))),
        response("compact", "The tool analysis was summarized.", 20),
        response(
            "answer",
            "Budget headroom was restored after compaction.",
            30,
        ),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        json!({"instances": []}),
    )])
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();
    service
        .set_session_turn_budget_policy(
            session.id,
            AgentTurnBudgetPolicy {
                maximum_cumulative_model_input_tokens: 100,
                ..AgentTurnBudgetPolicy::default()
            },
        )
        .unwrap();

    assert_eq!(
        service
            .start_turn(session.id, "run the tool loop".to_string())
            .await
            .unwrap()
            .final_answer,
        "Budget headroom was restored after compaction."
    );
    let snapshot = test_session_view(&service, session.id).unwrap();
    let compaction = snapshot.runtime.compaction.as_ref().unwrap();
    assert_eq!(compaction.trigger, AgentCompactionTrigger::Automatic);
    assert_eq!(
        compaction.reason,
        hanafubuki_agent_kernel::AgentCompactionReason::ContextLimit
    );
    assert_eq!(
        compaction.phase,
        hanafubuki_agent_kernel::AgentCompactionPhase::MidTurn
    );
    assert_eq!(snapshot.context_window.window_number, 1);
    assert_eq!(model.requests().len(), 3);
}

#[tokio::test]
async fn auto_compaction_does_not_run_below_threshold_or_without_usage() {
    let model = ScriptedModel::new([
        response("one", "first answer", 100),
        response("two", "second answer", 200),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut config = config();
    config.context_window_tokens = Some(10_000);
    config.auto_compact_threshold_percent = 80;
    let session = service.create_session(config).unwrap();

    // The first sampling has no prior usage; the second remains below policy.
    service
        .start_turn(session.id, "first".to_string())
        .await
        .unwrap();
    service
        .start_turn(session.id, "second".to_string())
        .await
        .unwrap();
    assert!(test_session_view(&service, session.id)
        .unwrap()
        .compaction
        .is_none());
    assert_eq!(model.requests().len(), 2);
    assert!(model
        .requests()
        .iter()
        .all(|request| !request.tools.is_empty()));
}

#[tokio::test]
async fn new_context_is_consumed_once_after_tool_followup_and_opens_a_new_window() {
    let model = ScriptedModel::new([
        new_context_tool_response(),
        response("compact", "fresh window requested", 12),
        response("answer", "continued in the fresh window", 20),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let outcome = service
        .start_turn(session.id, "continue with a fresh context".to_string())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "continued in the fresh window");
    let snapshot = test_session_view(&service, session.id).unwrap();
    let compaction = snapshot.runtime.compaction.as_ref().unwrap();
    assert_eq!(
        compaction.phase,
        hanafubuki_agent_kernel::AgentCompactionPhase::MidTurn
    );
    assert_eq!(
        compaction.reason,
        hanafubuki_agent_kernel::AgentCompactionReason::NewContextRequested
    );
    assert_eq!(snapshot.context_window.window_number, 1);
    assert!(!snapshot.context_window.new_context_requested);
    assert_eq!(model.requests().len(), 3);
}

/// Renders one fixed developer fragment into every request.
struct FixedPrompt;

impl hanafubuki_agent_contracts::PromptContributor for FixedPrompt {
    fn id(&self) -> &str {
        "fixed"
    }

    fn rank(&self) -> u32 {
        0
    }

    fn contribute(
        &self,
        _request: &hanafubuki_agent_contracts::PromptRequest<'_>,
    ) -> Vec<hanafubuki_agent_contracts::PromptFragment> {
        vec![hanafubuki_agent_contracts::PromptFragment {
            kind: ContextSectionKind::fixed("fixed"),
            version: None,
            content: "fixed prompt fragment".to_string(),
        }]
    }
}

/// A window compacted mid-turn carries its own initial context (mode and
/// checklist), and every prompt fragment is rendered again beside it, at the
/// head of the request as in a fresh window.
#[tokio::test]
async fn prompt_fragments_are_rendered_again_in_a_window_compacted_mid_turn() {
    use hanafubuki_agent_contracts::{
        ContextRendererRegistry, PromptContributorRegistry, Register,
    };
    use kardia_core::{Context, Inject, PluginDef, Service};

    let model = ScriptedModel::new([
        new_context_tool_response(),
        response("compact", "fresh window requested", 12),
        response("answer", "continued in the fresh window", 20),
        response("next", "next turn", 20),
    ]);
    let tools = FakeTools::new([]).await;
    tools
        .mount(
            &PluginDef::new(|ctx: Context, _| async move {
                ctx.get::<PromptContributorRegistry>()?
                    .register(Arc::new(FixedPrompt)
                        as Arc<dyn hanafubuki_agent_contracts::PromptContributor>)?;
                ctx.get::<ContextRendererRegistry>()?
                    .register(plain_renderer(
                        ContextSectionKind::fixed("fixed"),
                        ContextSlot::Capabilities,
                    ))?;
                Ok(())
            })
            .name("fixed-prompt")
            .inject(
                Inject::new()
                    .with(PromptContributorRegistry::NAME)
                    .with(ContextRendererRegistry::NAME),
            ),
        )
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(session.id, "continue with a fresh context".to_string())
        .await
        .unwrap();
    service
        .start_turn(session.id, "next".to_string())
        .await
        .unwrap();

    let has_fragment = |request: &ModelRequest| {
        request.input.iter().any(|item| {
            matches!(
                item,
                ModelInputItem::ContextFragment { content, .. } if content == "fixed prompt fragment"
            )
        })
    };
    let requests = model.requests();
    assert_eq!(requests.len(), 4);
    assert!(has_fragment(&requests[0]));
    assert!(!has_fragment(&requests[1]));
    assert!(requests[2].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ContextFragment { kind, .. } if kind == "canonical_initial_context"
    )));
    assert!(matches!(
        &requests[2].input[0],
        ModelInputItem::ContextFragment { content, .. } if content == "fixed prompt fragment"
    ));
    assert!(has_fragment(&requests[3]));
}

/// What [`SteppedMemoryPrompt`] prepared for one sampling step.
struct PreparedSummary(String);

/// Loads its "memory" per sampling step, the way the `memory` plugin reads
/// the published summary, and renders it as the memory context.
struct SteppedMemoryPrompt;

impl hanafubuki_agent_contracts::PromptContributor for SteppedMemoryPrompt {
    fn id(&self) -> &str {
        "stepped-memory"
    }

    /// Before the fixed fragment's contributor: the memory context still
    /// follows it, by its slot.
    fn rank(&self) -> u32 {
        0
    }

    fn prepare<'a>(
        &'a self,
        step: &'a hanafubuki_agent_contracts::PromptStep,
        _cancellation: CancellationToken,
    ) -> hanafubuki_agent_contracts::PromptPrepareFuture<'a> {
        Box::pin(async move {
            Ok(
                TurnExtensions::default().with(Arc::new(PreparedSummary(format!(
                    "memory prepared for step {} of turn {}",
                    step.sampling_step, step.turn_id
                )))),
            )
        })
    }

    fn contribute(
        &self,
        request: &hanafubuki_agent_contracts::PromptRequest<'_>,
    ) -> Vec<hanafubuki_agent_contracts::PromptFragment> {
        request
            .prepared
            .and_then(|prepared| prepared.get::<PreparedSummary>())
            .map(|summary| hanafubuki_agent_contracts::PromptFragment {
                kind: ContextSectionKind::fixed("memory"),
                version: Some("7".to_string()),
                content: summary.0.clone(),
            })
            .into_iter()
            .collect()
    }
}

/// The memory context is prepared afresh for every sampling step and
/// follows the other fragments, in a window compacted mid-turn as in any
/// other. Requests built outside a sampling step (the compaction itself)
/// carry none.
#[tokio::test]
async fn the_memory_context_is_prepared_per_step_and_follows_the_fragments_after_compaction() {
    use hanafubuki_agent_contracts::{
        ContextRendererRegistry, PromptContributor, PromptContributorRegistry, Register,
    };
    use hanafubuki_agent_model_api::ModelContextRole;
    use kardia_core::{Context, Inject, PluginDef, Service};

    let model = ScriptedModel::new([
        new_context_tool_response(),
        response("compact", "fresh window requested", 12),
        response("answer", "continued in the fresh window", 20),
        response("next", "next turn", 20),
    ]);
    let tools = FakeTools::new([]).await;
    tools
        .mount(
            &PluginDef::new(|ctx: Context, _| async move {
                let prompts = ctx.get::<PromptContributorRegistry>()?;
                prompts.register(Arc::new(SteppedMemoryPrompt) as Arc<dyn PromptContributor>)?;
                prompts.register(Arc::new(FixedPrompt) as Arc<dyn PromptContributor>)?;
                let renderers = ctx.get::<ContextRendererRegistry>()?;
                renderers.register(plain_renderer(
                    ContextSectionKind::fixed("memory"),
                    ContextSlot::Memory,
                ))?;
                renderers.register(plain_renderer(
                    ContextSectionKind::fixed("fixed"),
                    ContextSlot::Capabilities,
                ))?;
                Ok(())
            })
            .name("prompts")
            .inject(
                Inject::new()
                    .with(PromptContributorRegistry::NAME)
                    .with(ContextRendererRegistry::NAME),
            ),
        )
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();
    let first = service
        .start_turn(session.id, "continue with a fresh context".to_string())
        .await
        .unwrap();
    let second = service
        .start_turn(session.id, "next".to_string())
        .await
        .unwrap();

    let memory = |content: String| ModelInputItem::ContextFragment {
        role: ModelContextRole::Developer,
        kind: "memory".to_string(),
        content,
    };
    let is_memory = |item: &ModelInputItem| matches!(item, ModelInputItem::ContextFragment { kind, .. } if kind == "memory");
    let is_fixed = |item: &ModelInputItem| {
        matches!(
            item,
            ModelInputItem::ContextFragment { content, .. } if content == "fixed prompt fragment"
        )
    };
    let requests = model.requests();
    assert_eq!(requests.len(), 4);
    assert!(is_fixed(&requests[0].input[0]));
    assert_eq!(
        requests[0].input[1],
        memory(format!(
            "memory prepared for step 1 of turn {}",
            first.turn_id
        ))
    );
    assert!(!requests[1].input.iter().any(is_memory));
    assert!(is_fixed(&requests[2].input[0]));
    assert_eq!(
        requests[2].input[1],
        memory(format!(
            "memory prepared for step 2 of turn {}",
            first.turn_id
        ))
    );
    assert_eq!(
        requests[2]
            .input
            .iter()
            .filter(|item| is_memory(item))
            .count(),
        1
    );
    assert!(is_fixed(&requests[3].input[0]));
    assert_eq!(
        requests[3].input[1],
        memory(format!(
            "memory prepared for step 1 of turn {}",
            second.turn_id
        ))
    );
}

/// Prepares a large memory context for every step.
struct BulkyMemoryPrompt;

impl hanafubuki_agent_contracts::PromptContributor for BulkyMemoryPrompt {
    fn id(&self) -> &str {
        "bulky-memory"
    }

    fn rank(&self) -> u32 {
        0
    }

    fn prepare<'a>(
        &'a self,
        _step: &'a hanafubuki_agent_contracts::PromptStep,
        _cancellation: CancellationToken,
    ) -> hanafubuki_agent_contracts::PromptPrepareFuture<'a> {
        Box::pin(async {
            Ok(TurnExtensions::default()
                .with(Arc::new(PreparedSummary("remembered ".repeat(4_000)))))
        })
    }

    fn contribute(
        &self,
        request: &hanafubuki_agent_contracts::PromptRequest<'_>,
    ) -> Vec<hanafubuki_agent_contracts::PromptFragment> {
        request
            .prepared
            .and_then(|prepared| prepared.get::<PreparedSummary>())
            .map(|summary| hanafubuki_agent_contracts::PromptFragment {
                kind: ContextSectionKind::fixed("memory"),
                version: None,
                content: summary.0.clone(),
            })
            .into_iter()
            .collect()
    }
}

/// The admission estimate counts the request the step sends, with the
/// memory context the step prepared: history alone stays under the
/// threshold here, history and memory together do not.
#[tokio::test]
async fn the_admission_estimate_counts_the_steps_memory_context() {
    use hanafubuki_agent_contracts::{
        ContextRendererRegistry, PromptContributor, PromptContributorRegistry, Register,
    };
    use kardia_core::{Context, Inject, PluginDef, Service};

    let model = ScriptedModel::new([
        response("one", &"transient answer ".repeat(1_400), 10),
        response("compact", "The first exchange is summarized.", 10),
        response("two", "second answer", 10),
    ]);
    let tools = FakeTools::new([]).await;
    tools
        .mount(
            &PluginDef::new(|ctx: Context, _| async move {
                ctx.get::<PromptContributorRegistry>()?
                    .register(Arc::new(BulkyMemoryPrompt) as Arc<dyn PromptContributor>)?;
                ctx.get::<ContextRendererRegistry>()?
                    .register(plain_renderer(
                        ContextSectionKind::fixed("memory"),
                        ContextSlot::Memory,
                    ))?;
                Ok(())
            })
            .name("bulky-memory")
            .inject(
                Inject::new()
                    .with(PromptContributorRegistry::NAME)
                    .with(ContextRendererRegistry::NAME),
            ),
        )
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let mut config = config();
    config.context_window_tokens = Some(20_000);
    config.auto_compact_threshold_percent = 80;
    let session = service.create_session(config).unwrap();
    service.start_turn(session.id, "first").await.unwrap();

    let outcome = service.start_turn(session.id, "second").await.unwrap();

    assert_eq!(outcome.final_answer, "second answer");
    let snapshot = test_session_view(&service, session.id).unwrap();
    let compaction = snapshot.runtime.compaction.as_ref().unwrap();
    assert_eq!(
        compaction.phase,
        hanafubuki_agent_kernel::AgentCompactionPhase::PreTurn
    );
    assert_eq!(
        compaction.reason,
        hanafubuki_agent_kernel::AgentCompactionReason::ContextLimit
    );
    assert_eq!(model.requests().len(), 3);
}
