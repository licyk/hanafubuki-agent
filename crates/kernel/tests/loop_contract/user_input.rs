use super::*;

/// Proposes a plan, then implements it.
#[derive(Clone, Default)]
struct PlanThenImplementModel {
    attempts: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<ModelRequest>>>,
}

impl PlanThenImplementModel {
    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl ModelPort for PlanThenImplementModel {
    fn stream(
        &self,
        request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.requests.lock().unwrap().push(request);
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        let events = if attempt == 0 {
            deterministic_assistant_events(
                "<proposed_plan>\n# Plan\n\n- Apply the change.\n</proposed_plan>",
            )
        } else {
            deterministic_assistant_events("Implemented")
        };
        Box::pin(async move {
            Ok(Box::new(RecordedEventStream {
                events: events.into(),
            }) as ModelStream)
        })
    }
}

struct ProtocolIndependentPlanModel;

impl ModelPort for ProtocolIndependentPlanModel {
    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        let events = deterministic_assistant_events(
            "<proposed_plan>\n# Shared parser\n\n- Keep it provider-neutral.\n</proposed_plan>",
        );
        Box::pin(async move {
            Ok(Box::new(RecordedEventStream {
                events: events.into(),
            }) as ModelStream)
        })
    }
}

#[tokio::test]
async fn every_provider_protocol_uses_the_same_agent_plan_projection() {
    let service = AgentService::new(
        Arc::new(ProtocolIndependentPlanModel),
        FakeTools::new([]).await.registries(),
    );
    for protocol in [
        hanafubuki_agent_model_api::ModelApiProtocol::Responses,
        hanafubuki_agent_model_api::ModelApiProtocol::ChatCompletions,
        hanafubuki_agent_model_api::ModelApiProtocol::AnthropicMessages,
    ] {
        let mut session_config = config();
        session_config.provider_protocol = protocol;
        session_config.mode = hanafubuki_agent_kernel::AgentSessionMode::Plan;
        let session = service.create_session(session_config).unwrap();
        service.start_turn(session.id, "Plan it").await.unwrap();
        assert!(service
            .test_model_context(session.id)
            .unwrap()
            .iter()
            .any(|item| matches!(
                item,
                ConversationItem::AssistantMessage {
                    text,
                    model_text: Some(model_text),
                    proposed_plan: Some(plan),
                    ..
                } if text.is_empty()
                    && model_text.contains("<proposed_plan>")
                    && plan.text.contains("provider-neutral")
            )));
    }
}

#[tokio::test]
async fn plan_mode_rejects_the_default_task_checklist_without_mutating_state() {
    let task_model = TaskChecklistModel::default();
    let task_service = AgentService::new(
        Arc::new(task_model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut plan_mode = config();
    plan_mode.mode = hanafubuki_agent_kernel::AgentSessionMode::Plan;
    let plan_session = task_service.create_session(plan_mode).unwrap();
    // Plan mode does not offer the checklist: each call reads back the error
    // of a tool that does not exist, and the turn goes on.
    task_service
        .start_turn(plan_session.id, "Try the wrong checklist tool")
        .await
        .unwrap();
    let follow_up = task_model.requests().pop().unwrap();
    let rejected = follow_up
        .input
        .iter()
        .filter(|item| matches!(
            item,
            ModelInputItem::ToolOutput { output, .. }
                if output.is_error
                    && output.structured_content
                        == Some(json!({ "error": "no tool named `update_plan` is available in this step" }))
        ))
        .count();
    assert_eq!(rejected, 2);
    let plan_snapshot = task_service.snapshot(plan_session.id).unwrap();
    assert!(plan_snapshot.task_plan.is_none());
}

#[tokio::test]
async fn implementing_an_exact_plan_atomically_switches_to_default_and_starts_the_turn() {
    let model = PlanThenImplementModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut plan_config = config();
    plan_config.mode = hanafubuki_agent_kernel::AgentSessionMode::Plan;
    let session = service.create_session(plan_config).unwrap();
    service
        .start_turn(session.id, "Create a plan")
        .await
        .unwrap();

    let planned = test_session_view(&service, session.id).unwrap();
    assert_eq!(
        planned.mode,
        hanafubuki_agent_kernel::AgentSessionMode::Plan
    );
    assert_eq!(
        planned.turn.as_ref().unwrap().phase,
        AgentTurnPhase::Completed
    );
    let plan_message_id = planned
        .turn
        .as_ref()
        .unwrap()
        .final_answer_message_id
        .unwrap();
    assert!(planned.model_context.iter().any(|item| matches!(
        item,
        ConversationItem::AssistantMessage {
            message_id,
            text,
            model_text: Some(model_text),
            proposed_plan: Some(plan),
            ..
        } if *message_id == plan_message_id
            && text.is_empty()
            && model_text.contains("<proposed_plan>")
            && plan.text.contains("Apply the change")
    )));
    let plan_events = service.events_since(session.id, None).unwrap().events;
    let event_index = |predicate: &dyn Fn(&AgentEventKind) -> bool| {
        plan_events
            .iter()
            .position(|event| predicate(&event.kind))
            .expect("expected plan presentation event")
    };
    let started_index = event_index(&|kind| {
        matches!(
            kind,
            AgentEventKind::ProposedPlanStarted { message_id, .. }
                if *message_id == plan_message_id
        )
    });
    let delta_index = event_index(&|kind| {
        matches!(
            kind,
            AgentEventKind::ProposedPlanDelta { message_id, delta }
                if *message_id == plan_message_id && delta.contains("Apply the change")
        )
    });
    let plan_completed_index = event_index(&|kind| {
        matches!(
            kind,
            AgentEventKind::ProposedPlanCompleted { message_id }
                if *message_id == plan_message_id
        )
    });
    let assistant_completed_index = event_index(&|kind| {
        matches!(
            kind,
            AgentEventKind::AssistantMessageCompleted { message_id, .. }
                if *message_id == plan_message_id
        )
    });
    assert!(started_index < delta_index);
    assert!(delta_index < plan_completed_index);
    assert!(plan_completed_index < assistant_completed_index);
    let fork = service
        .fork_session_at_head(AgentSessionHeadForkRequest {
            source_session_id: session.id,
            source_generation: planned.generation,
            source_revision: planned.revision,
        })
        .unwrap();
    assert!(service
        .test_model_context(fork.session.id)
        .unwrap()
        .iter()
        .any(|item| matches!(
            item,
            ConversationItem::AssistantMessage {
                message_id,
                model_text: Some(model_text),
                proposed_plan: Some(plan),
                ..
            } if *message_id == plan_message_id
                && model_text.contains("<proposed_plan>")
                && plan.text.contains("Apply the change")
        )));
    let exact_request = AgentPlanImplementationRequest {
        session_id: session.id,
        generation: planned.generation,
        revision: planned.revision,
        message_id: plan_message_id,
    };
    for stale_request in [
        AgentPlanImplementationRequest {
            revision: AgentRevision::new(planned.revision.value() + 1),
            ..exact_request
        },
        AgentPlanImplementationRequest {
            generation: AgentGeneration::new(planned.generation.value() + 1),
            ..exact_request
        },
        AgentPlanImplementationRequest {
            message_id: AgentMessageId::new(plan_message_id.value() + 1),
            ..exact_request
        },
    ] {
        let stale =
            service.submit_plan_implementation_with_sink(stale_request, Arc::new(NoopEventSink));
        assert_eq!(stale, Err(AgentError::StaleRevision));
        assert_eq!(test_session_view(&service, session.id).unwrap(), planned);
    }
    let accepted = service
        .submit_plan_implementation_with_sink(
            AgentPlanImplementationRequest {
                session_id: session.id,
                generation: planned.generation,
                revision: planned.revision,
                message_id: plan_message_id,
            },
            Arc::new(NoopEventSink),
        )
        .unwrap();
    let implemented = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let current = test_session_view(&service, session.id).unwrap();
            if current.turn.as_ref().is_some_and(|turn| {
                turn.generation == accepted.generation && turn.phase == AgentTurnPhase::Completed
            }) {
                break current;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("implementation turn should complete");
    assert_eq!(
        implemented.mode,
        hanafubuki_agent_kernel::AgentSessionMode::Default
    );
    assert!(implemented.task_plan.is_none());
    assert!(implemented.model_context.iter().any(|item| matches!(
        item,
        ConversationItem::UserMessage { text, .. } if text == "Implement the plan."
    )));
    let implemented_final_id = implemented
        .turn
        .as_ref()
        .unwrap()
        .final_answer_message_id
        .unwrap();
    assert!(implemented.model_context.iter().any(|item| matches!(
        item,
        ConversationItem::AssistantMessage { message_id, text, .. }
            if *message_id == implemented_final_id && text == "Implemented"
    )));
    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input.iter().any(|item| matches!(
        item,
        ModelInputItem::AssistantMessage { text, .. }
            if text.contains("<proposed_plan>")
                && text.contains("Apply the change")
                && text.contains("</proposed_plan>")
    )));
    assert!(!requests[1]
        .instructions
        .contains("# Plan Mode (Conversational)"));
    assert!(requests[1].tools.iter().any(|tool| {
        tool.name()
            .is_some_and(|name| name.flattened_alias() == "control_instance")
    }));
    let replay = service.submit_plan_implementation_with_sink(
        AgentPlanImplementationRequest {
            session_id: session.id,
            generation: implemented.generation,
            revision: implemented.revision,
            message_id: plan_message_id,
        },
        Arc::new(NoopEventSink),
    );
    assert!(
        matches!(replay, Err(AgentError::InvalidInput(message)) if message.contains("Plan mode"))
    );
}

#[tokio::test]
async fn structured_user_input_pauses_exact_turn_and_resumes_with_tool_output() {
    let question_arguments = json!({
        "questions": [{
            "id": "scope",
            "header": "Scope",
            "question": "Choose a scope",
            "options": [
                { "id": "focused", "label": "Focused", "description": "Only this feature" },
                { "id": "broad", "label": "Broad", "description": "Include cleanup" }
            ]
        }]
    });
    let first = request(vec![ModelInputItem::user_text("plan it")]);
    let second = request(vec![
        ModelInputItem::user_text("plan it"),
        ModelInputItem::ToolCall {
            call_id: "call-question".to_string(),
            namespace: None,
            name: "request_user_input".to_string(),
            arguments: question_arguments.clone(),
        },
        ModelInputItem::ToolOutput {
            call_id: "call-question".to_string(),
            output: json!({ "answers": [{ "questionId": "scope", "optionId": "focused" }] }).into(),
        },
    ]);
    let question_events = vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "response-question".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "item-question".to_string(),
                    call_id: "call-question".to_string(),
                    namespace: None,
                    name: "request_user_input".to_string(),
                    arguments: question_arguments.clone(),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "item-question".to_string(),
                    call_id: "call-question".to_string(),
                    namespace: None,
                    name: "request_user_input".to_string(),
                    arguments: question_arguments,
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response-question".to_string(),
            usage: None,
            end_turn: Some(false),
        }),
    ];
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first, question_events),
        FakeModelExchange::new(second, deterministic_assistant_events("focused plan ready")),
    ]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    let running = service.clone();
    let turn_task =
        tokio::spawn(async move { running.start_turn(session.id, "plan it".to_string()).await });

    let (turn, pending) = loop {
        let snapshot = test_session_view(&service, session.id).unwrap();
        if let (Some(turn), Some(pending)) =
            (snapshot.runtime.turn, snapshot.runtime.pending_user_input)
        {
            break (turn, pending);
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    assert_eq!(turn.phase, AgentTurnPhase::WaitingForUserInput);
    assert!(tools.calls().is_empty());
    assert_eq!(pending.call_id, "call-question");
    assert_eq!(
        service.resolve_user_input(
            session.id,
            turn.id,
            session.generation,
            pending.request_id,
            vec![hanafubuki_agent_kernel::AgentUserInputAnswer {
                question_id: "scope".to_string(),
                option_id: "focused".to_string(),
            }],
        ),
        Err(AgentError::StaleUserInput)
    );
    service
        .resolve_user_input(
            session.id,
            turn.id,
            turn.generation,
            pending.request_id,
            vec![hanafubuki_agent_kernel::AgentUserInputAnswer {
                question_id: "scope".to_string(),
                option_id: "focused".to_string(),
            }],
        )
        .unwrap();

    let outcome = turn_task.await.unwrap().unwrap();
    assert_eq!(outcome.final_answer, "focused plan ready");
    let snapshot = test_session_view(&service, session.id).unwrap();
    assert!(snapshot.pending_user_input.is_none());
    assert_eq!(
        snapshot.runtime.turn.unwrap().phase,
        AgentTurnPhase::Completed
    );
    assert!(tools.calls().is_empty());
}
