use super::*;

/// Turn extensions that make `skill` selectable with `guidance` as its
/// instructions.
fn selecting(skill: &SkillId, guidance: &str) -> TurnExtensions {
    TurnExtensions::default().with(Arc::new(SelectableSkills {
        skills: vec![SelectableSkill {
            id: skill.as_str().to_string(),
            name: "Workflow".to_string(),
            context: format!("<skill>\n<name>Workflow</name>\n{guidance}\n</skill>"),
        }],
        ..Default::default()
    }))
}

#[tokio::test]
async fn compaction_never_splits_skill_context_from_its_user_message() {
    let model = ScriptedModel::new([response("one", "answer", 10)]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let skill = SkillId::new(SkillScope::User, "workflow").unwrap();
    let selectable = selecting(&skill, "untrusted guidance");
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(
            session.id,
            AgentTurnInput {
                text: "use it".to_string(),
                images: Vec::new(),
                skills: vec![AgentSkillRef { skill }],
                turn: selectable,
                world_state: hanafubuki_agent_kernel::AgentWorldStateSnapshot {
                    sections: vec![hanafubuki_agent_kernel::AgentWorldStateSection::new(
                        hanafubuki_agent_kernel::ContextSectionKind::fixed("host_location"),
                        r#"{"page":"instance chooser"}"#.to_string(),
                    )],
                },
                permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::Manual,
                origin: Default::default(),
                provisional_title: None,
            },
        )
        .await
        .unwrap();
    let before = test_session_view(&service, session.id).unwrap();
    assert_eq!(before.model_context.len(), 5);
    assert!(matches!(
        before.model_context[1],
        ConversationItem::WorldStateUpdate { .. }
    ));
    assert!(matches!(
        before.model_context[2],
        ConversationItem::SkillContext { .. }
    ));
    assert!(matches!(
        service
            .compact_session(session.id, before.generation, before.revision)
            .await,
        Err(AgentError::Model(_))
    ));
}

#[tokio::test]
async fn compaction_then_new_turn_does_not_reinstall_an_old_skill() {
    let model = ScriptedModel::new([
        response("first", "first answer", 10),
        response("second", "second answer", 10),
        response("compact", "summary without retained Skill instructions", 10),
        response("third", "third answer", 10),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let skill = SkillId::new(SkillScope::User, "workflow").unwrap();
    let catalog = selecting(&skill, "old workflow");
    let mut first = AgentTurnInput::from("use the workflow");
    first.skills.push(AgentSkillRef { skill });
    first.turn = catalog;
    service.start_turn(session.id, first).await.unwrap();
    service.start_turn(session.id, "second").await.unwrap();

    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    service.start_turn(session.id, "third").await.unwrap();

    let requests = model.requests();
    assert_eq!(requests.len(), 4);
    assert!(!requests[3].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ContextFragment { kind, .. }
            if kind == "skill_catalog" || kind == "skill_instructions"
    )));
    assert!(!requests[3]
        .input
        .iter()
        .any(|item| format!("{item:?}").contains("old workflow")));
    let context = service.test_model_context(session.id).unwrap();
    let latest_turn = context
        .iter()
        .rposition(|item| matches!(item, ConversationItem::TurnStart { .. }))
        .unwrap();
    assert!(!context[latest_turn..]
        .iter()
        .any(|item| matches!(item, ConversationItem::SkillContext { .. })));
}

/// A `tool_search` that selects the deferred `configure_instance`.
fn tool_search_response() -> ScriptedResponse {
    let call = ModelToolCall {
        item_id: "search-item".to_string(),
        call_id: "search-call".to_string(),
        namespace: None,
        name: "tool_search".to_string(),
        arguments: json!({ "query": "configure instance" }),
        argument_error: None,
    };
    ScriptedResponse::Events(vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "search".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall { call: call.clone() },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { call },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "search".to_string(),
            usage: None,
            end_turn: Some(false),
        }),
    ])
}

fn offers(request: &ModelRequest, tool: &str) -> bool {
    request.tools.iter().any(|spec| {
        spec.name()
            .is_some_and(|name| name.flattened_alias() == tool)
    })
}

/// A selection is a history item: the planner offers the tool in the next
/// turn too, and stops once a compaction replaced the history it was in.
#[tokio::test]
async fn a_tool_search_selection_lasts_until_the_next_compaction() {
    let model = ScriptedModel::new([
        tool_search_response(),
        response("found", "found it", 10),
        response("second", "second answer", 10),
        response("compact", "summary", 10),
        response("third", "third answer", 10),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "first").await.unwrap();
    let context = service.test_model_context(session.id).unwrap();
    let selection = context
        .iter()
        .position(|item| {
            matches!(item, ConversationItem::ToolSelection { call_id, tools }
                if call_id == "search-call" && tools.len() == 1
                    && tools[0].tool_id.as_str() == "configure_instance")
        })
        .expect("the search committed its selection");
    assert!(matches!(
        &context[selection - 1],
        ConversationItem::ToolOutput { call_id, .. } if call_id == "search-call"
    ));
    service.start_turn(session.id, "second").await.unwrap();

    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    assert!(!service
        .test_model_context(session.id)
        .unwrap()
        .iter()
        .any(|item| matches!(item, ConversationItem::ToolSelection { .. })));
    service.start_turn(session.id, "third").await.unwrap();

    let requests = model.requests();
    assert_eq!(requests.len(), 5);
    assert!(!offers(&requests[0], "configure_instance"));
    assert!(offers(&requests[1], "configure_instance"));
    assert!(offers(&requests[2], "configure_instance"));
    assert!(!offers(&requests[4], "configure_instance"));
    assert!(offers(&requests[4], "tool_search"));
}

#[tokio::test]
async fn manual_compaction_replaces_a_safe_prefix_and_commits_usage_atomically() {
    let model = ScriptedModel::new([
        response("one", "first answer", 10),
        response("two", "second answer", 12),
        response("compact", "The first exchange established the baseline.", 8),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(session.id, "first".to_string())
        .await
        .unwrap();
    service
        .start_turn(session.id, "second".to_string())
        .await
        .unwrap();
    let before = test_session_view(&service, session.id).unwrap();

    let outcome = service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    let after = test_session_view(&service, session.id).unwrap();

    assert_eq!(outcome.compaction.status, AgentCompactionStatus::Completed);
    assert_eq!(outcome.compaction.trigger, AgentCompactionTrigger::Manual);
    assert_eq!(outcome.compaction.source_revision, before.revision);
    assert!(matches!(
        after.model_context.last().unwrap(),
        ConversationItem::CompactionSummary {
            summary,
            source_revision,
            compacted_item_count,
        } if summary.contains("baseline")
            && *source_revision == before.revision
            && *compacted_item_count == before.model_context.len()
    ));
    assert!(
        matches!(&after.model_context[0], ConversationItem::UserMessage { text, .. } if text == "first")
    );
    assert!(
        matches!(&after.model_context[1], ConversationItem::UserMessage { text, .. } if text == "second")
    );
    assert_eq!(after.model_context.len(), 3);
    assert_eq!(after.usage.unwrap().cumulative_input_tokens, 30);
    assert!(model.requests()[2].tools.is_empty());
    assert!(!model.requests()[2].parallel_tool_calls);
    let lifecycle = service
        .events_since(session.id, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::CompactionUpdated { compaction } => {
                assert!(event.turn_id.is_none());
                Some(compaction.status)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        lifecycle,
        vec![
            AgentCompactionStatus::Reserved,
            AgentCompactionStatus::Running,
            AgentCompactionStatus::ReadyToInstall,
            AgentCompactionStatus::Completed,
        ]
    );
    assert_eq!(
        service
            .compact_session(session.id, before.generation, before.revision)
            .await,
        Err(AgentError::StaleRevision)
    );
}

#[tokio::test]
async fn compaction_tool_call_fails_without_tool_execution_or_history_replacement() {
    let forbidden = ScriptedResponse::Events(vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "compact".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "item-call".to_string(),
                    call_id: "call".to_string(),
                    namespace: None,
                    name: "list_instances".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "item-call".to_string(),
                    call_id: "call".to_string(),
                    namespace: None,
                    name: "list_instances".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
    ]);
    let model = ScriptedModel::new([
        response("one", "answer", 10),
        response("two", "another answer", 10),
        forbidden,
    ]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(session.id, "first ".repeat(200))
        .await
        .unwrap();
    service
        .start_turn(session.id, "second".to_string())
        .await
        .unwrap();
    let before = test_session_view(&service, session.id).unwrap();

    let error = service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap_err();
    assert!(matches!(error, AgentError::Protocol(_)));
    let after = test_session_view(&service, session.id).unwrap();
    assert_eq!(after.model_context, before.model_context);
    assert_eq!(
        after.runtime.compaction.as_ref().unwrap().status,
        AgentCompactionStatus::Failed
    );
    assert!(tools.calls().is_empty());
}

#[tokio::test]
async fn local_compaction_rejects_empty_summary_and_early_eof() {
    for invalid in [
        response("compact", "   ", 4),
        ScriptedResponse::Events(vec![Ok(ModelEvent::ResponseStarted {
            response_id: "compact-eof".to_string(),
            metadata: ModelResponseMetadata::default(),
        })]),
    ] {
        let model = ScriptedModel::new([response("answer", "baseline", 4), invalid]);
        let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
        let session = service.create_session(config()).unwrap();
        service.start_turn(session.id, "hello").await.unwrap();
        let before = test_session_view(&service, session.id).unwrap();
        assert!(matches!(
            service
                .compact_session(session.id, before.generation, before.revision)
                .await,
            Err(AgentError::Protocol(_))
        ));
        let after = test_session_view(&service, session.id).unwrap();
        assert_eq!(after.context_window.window_number, 0);
        assert_eq!(after.model_context, before.model_context);
        assert_eq!(
            after.runtime.compaction.as_ref().unwrap().status,
            AgentCompactionStatus::Failed
        );
    }
}

#[tokio::test]
async fn compaction_boundary_never_splits_a_tool_call_from_its_output() {
    let model = ScriptedModel::new([
        tool_response(),
        response("answer", "tool result accepted", 12),
        response("second", "second turn", 10),
        response(
            "compact",
            "The request initiated a bounded instance query.",
            8,
        ),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        json!({"instances": []}),
    )])
    .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(session.id, "list".to_string())
        .await
        .unwrap();
    service
        .start_turn(session.id, "continue".to_string())
        .await
        .unwrap();
    let before = test_session_view(&service, session.id).unwrap();

    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    let after = test_session_view(&service, session.id).unwrap();
    assert!(!after.model_context.iter().any(|item| matches!(
        item,
        ConversationItem::ToolCall { .. } | ConversationItem::ToolOutput { .. }
    )));
    assert_eq!(after.model_context.len(), 3);
}

#[tokio::test]
async fn cancellation_retains_history_and_same_session_turn_is_excluded() {
    let model = ScriptedModel::new([
        response("one", "answer", 10),
        response("two", "second answer", 10),
        ScriptedResponse::WaitForCancellation,
    ]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(session.id, "first ".repeat(1_000))
        .await
        .unwrap();
    service
        .start_turn(session.id, "second".to_string())
        .await
        .unwrap();
    let before = test_session_view(&service, session.id).unwrap();
    let generation = before.generation;
    let revision = before.revision;
    let executing = service.clone();
    let task = tokio::spawn(async move {
        executing
            .compact_session(session.id, generation, revision)
            .await
    });
    let compaction = loop {
        let snapshot = test_session_view(&service, session.id).unwrap();
        if let Some(compaction) = snapshot.runtime.compaction {
            if compaction.status == AgentCompactionStatus::Running {
                break compaction;
            }
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    assert_eq!(
        service
            .start_turn(session.id, "cannot overlap".to_string())
            .await,
        Err(AgentError::CompactionAlreadyRunning)
    );
    service
        .cancel_compaction(session.id, compaction.id, compaction.generation)
        .unwrap();
    assert_eq!(task.await.unwrap(), Err(AgentError::Cancelled));
    let after = test_session_view(&service, session.id).unwrap();
    assert_eq!(after.model_context, before.model_context);
    assert_eq!(
        after.runtime.compaction.as_ref().unwrap().status,
        AgentCompactionStatus::Cancelled
    );
}

#[tokio::test]
async fn different_sessions_can_compact_independently() {
    let model = ScriptedModel::new([
        response("one", "answer one", 10),
        response("two", "answer two", 10),
        response("three", "answer three", 10),
        response("four", "answer four", 10),
        ScriptedResponse::WaitForCancellation,
        ScriptedResponse::WaitForCancellation,
    ]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let first = service.create_session(config()).unwrap();
    let second = service.create_session(config()).unwrap();
    service
        .start_turn(first.id, "first".to_string())
        .await
        .unwrap();
    service
        .start_turn(second.id, "second".to_string())
        .await
        .unwrap();
    service
        .start_turn(first.id, "first again".to_string())
        .await
        .unwrap();
    service
        .start_turn(second.id, "second again".to_string())
        .await
        .unwrap();
    let first_before = test_session_view(&service, first.id).unwrap();
    let second_before = test_session_view(&service, second.id).unwrap();
    let first_service = service.clone();
    let first_task = tokio::spawn(async move {
        first_service
            .compact_session(first.id, first_before.generation, first_before.revision)
            .await
    });
    let second_service = service.clone();
    let second_task = tokio::spawn(async move {
        second_service
            .compact_session(second.id, second_before.generation, second_before.revision)
            .await
    });

    let (first_compaction, second_compaction) = loop {
        let first_compaction = test_session_view(&service, first.id)
            .unwrap()
            .runtime
            .compaction;
        let second_compaction = test_session_view(&service, second.id)
            .unwrap()
            .runtime
            .compaction;
        if let (Some(first), Some(second)) = (first_compaction, second_compaction) {
            if first.status == AgentCompactionStatus::Running
                && second.status == AgentCompactionStatus::Running
            {
                break (first, second);
            }
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    service
        .cancel_compaction(first.id, first_compaction.id, first_compaction.generation)
        .unwrap();
    service
        .cancel_compaction(
            second.id,
            second_compaction.id,
            second_compaction.generation,
        )
        .unwrap();
    assert_eq!(first_task.await.unwrap(), Err(AgentError::Cancelled));
    assert_eq!(second_task.await.unwrap(), Err(AgentError::Cancelled));
}

#[tokio::test]
async fn local_compaction_retries_before_output_then_trims_the_oldest_complete_turn() {
    let context_error = ModelError::ContextWindowExceeded {
        status: Some(400),
        code: Some("context_length_exceeded".to_string()),
        message: "maximum context length exceeded".to_string(),
        request_id: None,
    };
    let model = ScriptedModel::new([
        response("first", "old answer", 10),
        response("second", "recent answer", 10),
        ScriptedResponse::Events(vec![Err(ModelError::StreamInterrupted(
            "connection reset before output".to_string(),
        ))]),
        ScriptedResponse::Events(vec![Err(context_error)]),
        response("compact", "only recent context remains", 10),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "old user").await.unwrap();
    service.start_turn(session.id, "recent user").await.unwrap();
    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();

    let requests = model.requests();
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[2].input, requests[3].input);
    assert!(requests[4].input.iter().any(|item| {
        matches!(item, ModelInputItem::UserMessage { content } if format!("{content:?}").contains("recent user"))
    }));
    assert!(!requests[4].input.iter().any(|item| {
        matches!(item, ModelInputItem::UserMessage { content } if format!("{content:?}").contains("old user"))
    }));
    assert!(matches!(
        test_session_view(&service, session.id)
            .unwrap()
            .model_context
            .last(),
        Some(ConversationItem::CompactionSummary { summary, .. })
            if summary.contains("only recent context remains")
    ));
}
