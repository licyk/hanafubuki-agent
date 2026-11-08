use super::*;

#[tokio::test]
async fn continuation_hook_context_is_committed_before_resampling() {
    let provider_id = AgentContinuationHookProviderId::parse("test-provider".to_string()).unwrap();
    let hook_id = AgentContinuationHookId::parse("continue-once".to_string()).unwrap();
    let generation = AgentContinuationHookGeneration([7; 32]);
    let first = request(vec![ModelInputItem::user_text("inspect")]);
    let second = request(vec![
        ModelInputItem::user_text("inspect"),
        ModelInputItem::AssistantMessage {
            phase: Some(ModelMessagePhase::FinalAnswer),
            text: "draft".to_string(),
        },
        ModelInputItem::ContinuationContext {
            provider_id: provider_id.as_str().to_string(),
            hook_id: hook_id.as_str().to_string(),
            generation: generation.0,
            content: "include readiness".to_string(),
        },
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first, deterministic_assistant_events("draft")),
        FakeModelExchange::new(second, deterministic_assistant_events("ready")),
    ]);
    let tools = FakeTools::new([]).await;
    tools
        .mount(&registering::<ContinuationHookRegistry>(
            "continue-once",
            vec![AgentContinuationHookRegistration {
                provider_id: provider_id.clone(),
                hook_id: hook_id.clone(),
                generation,
                priority: 10,
                failure_policy: AgentContinuationHookFailurePolicy::FailTurn,
                hook: Arc::new(ContinueOnceHook::default()),
            }],
        ))
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();

    assert_eq!(
        service
            .start_turn(session.id, "inspect".to_string())
            .await
            .unwrap()
            .final_answer,
        "ready"
    );
    assert!(test_session_view(&service, session.id)
        .unwrap()
        .model_context
        .iter()
        .any(|item| {
            matches!(
                item,
                ConversationItem::ContinuationContext {
                    provider_id: actual_provider,
                    hook_id: actual_hook,
                    generation: actual_generation,
                    content,
                } if actual_provider == &provider_id
                    && actual_hook == &hook_id
                    && actual_generation == &generation
                    && content == "include readiness"
            )
        }));
}

/// Waits in `evaluate` until its plugin's stop cancels it.
struct WaitsForItsPluginToStop {
    entered: Arc<Barrier>,
}

impl AgentContinuationHook for WaitsForItsPluginToStop {
    fn evaluate<'a>(
        &'a self,
        _context: AgentContinuationContext,
        cancellation: CancellationToken,
    ) -> AgentContinuationHookFuture<'a> {
        Box::pin(async move {
            self.entered.wait().await;
            while !cancellation.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(hanafubuki_agent_contracts::AgentContinuationHookError::Cancelled)
        })
    }
}

#[tokio::test]
async fn a_hook_stopped_with_its_plugin_no_longer_holds_the_turn() {
    let model = FakeModelTransport::new([FakeModelExchange::new(
        request(vec![ModelInputItem::user_text("inspect")]),
        deterministic_assistant_events("draft"),
    )]);
    let entered = Arc::new(Barrier::new(2));
    let tools = FakeTools::new([]).await;
    let hooks = tools
        .mount(&registering::<ContinuationHookRegistry>(
            "stopping-hook",
            vec![AgentContinuationHookRegistration {
                provider_id: AgentContinuationHookProviderId::parse("test-provider".to_string())
                    .unwrap(),
                hook_id: AgentContinuationHookId::parse("stopping".to_string()).unwrap(),
                generation: AgentContinuationHookGeneration([3; 32]),
                priority: 1,
                // Even a hook that fails the turn on failure: its plugin
                // stopping removes its say instead.
                failure_policy: AgentContinuationHookFailurePolicy::FailTurn,
                hook: Arc::new(WaitsForItsPluginToStop {
                    entered: entered.clone(),
                }),
            }],
        ))
        .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    let executing = service.clone();
    let turn = tokio::spawn(async move {
        executing
            .start_turn(session.id, "inspect".to_string())
            .await
    });
    entered.wait().await;
    // The stop waits out the drain grace, then cancels the running hook.
    hooks.dispose().await.unwrap();

    assert_eq!(turn.await.unwrap().unwrap().final_answer, "draft");
    let events = service.events_since(session.id, None).unwrap().events;
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        AgentEventKind::ContinuationHookCompleted { hook_id, outcome, .. }
            if hook_id == "stopping"
                && *outcome == hanafubuki_agent_kernel::AgentContinuationHookOutcome::IgnoredFailure
    )));
}
