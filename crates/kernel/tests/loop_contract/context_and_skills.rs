use super::*;

#[tokio::test]
async fn task_checklist_updates_are_ordered_injected_and_scoped_to_one_turn() {
    let model = TaskChecklistModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();

    service
        .start_turn(session.id, "Implement the change")
        .await
        .unwrap();

    let completed = test_session_view(&service, session.id).unwrap();
    let plan = completed
        .task_plan
        .as_ref()
        .expect("task checklist retained after terminal state");
    assert_eq!(plan.turn_id, completed.turn.as_ref().unwrap().id);
    assert_eq!(plan.generation, completed.generation);
    assert_eq!(plan.revision, 2);
    assert_eq!(plan.explanation.as_deref(), Some("Projection is next"));
    assert_eq!(
        plan.plan[0].status,
        hanafubuki_agent_kernel::AgentTaskPlanStepStatus::Completed
    );
    assert_eq!(
        plan.plan[1].status,
        hanafubuki_agent_kernel::AgentTaskPlanStepStatus::InProgress
    );
    let commentary_index = completed
        .model_context
        .iter()
        .position(|item| {
            matches!(
                item,
                ConversationItem::AssistantMessage { text, .. }
                    if text == "Tracking implementation progress"
            )
        })
        .unwrap();
    let first_plan_call_index = completed
        .model_context
        .iter()
        .position(|item| {
            matches!(
                item,
                ConversationItem::ToolCall { name, .. } if name == "update_plan"
            )
        })
        .unwrap();
    assert!(commentary_index < first_plan_call_index);

    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    assert!(matches!(
        requests[1].input.last(),
        Some(ModelInputItem::ContextFragment {
            role: ModelContextRole::User,
            kind,
            content,
        }) if kind == "task_plan"
            && content.contains("Explanation: Projection is next")
            && content.contains("- [completed] Inspect the contract")
            && content.contains("- [in_progress] Update the projection")
            && !content.contains("Starting implementation")
    ));

    let forked = service
        .fork_session_before_turn(hanafubuki_agent_kernel::AgentSessionForkRequest {
            source_session_id: session.id,
            source_generation: completed.generation,
            source_revision: completed.revision,
            before_turn_id: completed.turn.as_ref().unwrap().id,
        })
        .unwrap();
    assert!(forked.session.task_plan.is_none());

    service.start_turn(session.id, "Continue").await.unwrap();
    assert!(service.snapshot(session.id).unwrap().task_plan.is_none());
}

#[tokio::test]
async fn a_host_context_is_committed_and_rendered_before_the_user_message() {
    let content = r#"{"page":"instance alpha"}"#;
    let expected = request(vec![
        host_context_fragment(content),
        ModelInputItem::user_text("why did launch fail?"),
    ]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("checking alpha"),
    )]);
    let tools = FakeTools::new([]).await;
    let _host = tools.mount(&host_context_renderer()).await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(
            session.id,
            AgentTurnInput {
                text: "why did launch fail?".to_string(),
                images: Vec::new(),
                skills: Vec::new(),
                turn: Default::default(),
                world_state: hanafubuki_agent_kernel::AgentWorldStateSnapshot {
                    sections: vec![hanafubuki_agent_kernel::AgentWorldStateSection::new(
                        HOST_LOCATION,
                        content.to_string(),
                    )],
                },
                permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::Manual,
                origin: Default::default(),
                provisional_title: None,
            },
        )
        .await
        .unwrap();
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert!(matches!(history[0], ConversationItem::TurnStart { .. }));
    assert!(matches!(
        &history[1],
        ConversationItem::WorldStateUpdate { updates }
            if matches!(updates.as_slice(), [hanafubuki_agent_kernel::AgentWorldStateUpdate::Replace {
                kind,
                content: committed,
                ..
            }] if *kind == HOST_LOCATION && committed == content)
    ));
    assert!(matches!(history[2], ConversationItem::UserMessage { .. }));
    assert!(matches!(
        history[3],
        ConversationItem::AssistantMessage { .. }
    ));
}

#[tokio::test]
async fn exact_and_dollar_skill_selection_commit_one_context_atomically() {
    let snapshot = selectable_skills_fixture();
    let expected = request(vec![
        skill_instructions_fragment("user:workflow", "user", &snapshot.skills[0].context),
        ModelInputItem::user_text("$workflow do it"),
    ]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("done"),
    )]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(
            session.id,
            AgentTurnInput {
                text: "$workflow do it".to_string(),
                images: Vec::new(),
                skills: vec![AgentSkillRef {
                    skill: skill_id(SkillScope::User, "workflow"),
                }],
                turn: selecting(snapshot.clone()),
                world_state: Default::default(),
                permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::Manual,
                origin: Default::default(),
                provisional_title: None,
            },
        )
        .await
        .unwrap();
    let committed = test_session_view(&service, session.id).unwrap();
    assert!(matches!(
        committed.model_context[0],
        ConversationItem::TurnStart { .. }
    ));
    assert!(matches!(
        committed.model_context[1],
        ConversationItem::SkillContext { .. }
    ));
    assert!(matches!(
        committed.model_context[2],
        ConversationItem::UserMessage { .. }
    ));
    assert!(matches!(
        committed.model_context[3],
        ConversationItem::AssistantMessage { .. }
    ));

    let before = committed.model_context.clone();
    let stale = service
        .start_turn(
            session.id,
            AgentTurnInput {
                text: "stale".to_string(),
                images: Vec::new(),
                skills: vec![AgentSkillRef {
                    skill: skill_id(SkillScope::Repo, "workflow"),
                }],
                turn: selecting(snapshot.clone()),
                world_state: Default::default(),
                permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::Manual,
                origin: Default::default(),
                provisional_title: None,
            },
        )
        .await;
    assert_eq!(stale, Err(AgentError::StaleGeneration));
    assert_eq!(
        test_session_view(&service, session.id)
            .unwrap()
            .model_context,
        before
    );

    let over_limit = service
        .start_turn(
            session.id,
            AgentTurnInput {
                text: "too many".to_string(),
                images: Vec::new(),
                skills: (0..9)
                    .map(|index| AgentSkillRef {
                        skill: skill_id(SkillScope::User, &format!("skill-{index}")),
                    })
                    .collect(),
                turn: selecting(snapshot.clone()),
                world_state: Default::default(),
                permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::Manual,
                origin: Default::default(),
                provisional_title: None,
            },
        )
        .await;
    assert_eq!(
        over_limit,
        Err(AgentError::LimitExceeded {
            resource: "selected Skills per turn",
            limit: 8,
        })
    );
    assert_eq!(
        test_session_view(&service, session.id)
            .unwrap()
            .model_context,
        before
    );
}

/// Renders the turn's [`Note`] as a prompt fragment, the way the `skills`
/// plugin renders its catalog.
struct NotePrompt;

struct Note(&'static str);

impl PromptContributor for NotePrompt {
    fn id(&self) -> &str {
        "note"
    }

    fn rank(&self) -> u32 {
        0
    }

    fn contribute(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        request
            .turn
            .get::<Note>()
            .map(|note| PromptFragment {
                kind: ContextSectionKind::fixed("note"),
                version: Some(format!("{:?}", request.context_window_tokens)),
                content: note.0.to_string(),
            })
            .into_iter()
            .collect()
    }
}

#[tokio::test]
async fn prompt_fragments_render_each_turns_frozen_extensions_before_the_conversation() {
    let model = RecordingAssistantModel::default();
    let tools = FakeTools::new([]).await;
    tools
        .mount(&registering::<PromptContributorRegistry>(
            "note-prompt",
            vec![Arc::new(NotePrompt) as Arc<dyn PromptContributor>],
        ))
        .await;
    tools
        .mount(&registering::<ContextRendererRegistry>(
            "note-renderer",
            vec![plain_renderer(
                ContextSectionKind::fixed("note"),
                ContextSlot::Capabilities,
            )],
        ))
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let mut session_config = config();
    session_config.context_window_tokens = Some(64_000);
    let session = service.create_session(session_config).unwrap();

    let mut first = AgentTurnInput::from("first");
    first.turn = TurnExtensions::default().with(Arc::new(Note("first-turn note")));
    service.start_turn(session.id, first).await.unwrap();
    let mut second = AgentTurnInput::from("second");
    second.turn = TurnExtensions::default().with(Arc::new(Note("second-turn note")));
    service.start_turn(session.id, second).await.unwrap();
    service.start_turn(session.id, "third").await.unwrap();

    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    for (request, note) in requests.iter().zip(["first-turn note", "second-turn note"]) {
        assert_eq!(
            request.input[0],
            ModelInputItem::ContextFragment {
                role: ModelContextRole::Developer,
                kind: "note".to_string(),
                content: note.to_string(),
            }
        );
        assert_eq!(
            request
                .input
                .iter()
                .filter(|item| matches!(
                    item,
                    ModelInputItem::ContextFragment { kind, .. } if kind == "note"
                ))
                .count(),
            1
        );
    }
    // A turn whose extensions lack the note gets no fragment.
    assert!(!requests[2].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ContextFragment { kind, .. } if kind == "note"
    )));
}

/// A section is shown only by its kind's renderer: a fragment whose kind has
/// none (its plugin registered no renderer, or stopped) is left out, and the
/// turn goes on.
#[tokio::test]
async fn a_fragment_without_a_renderer_is_left_out_of_the_request() {
    let model = RecordingAssistantModel::default();
    let tools = FakeTools::new([]).await;
    tools
        .mount(&registering::<PromptContributorRegistry>(
            "note-prompt",
            vec![Arc::new(NotePrompt) as Arc<dyn PromptContributor>],
        ))
        .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();

    let mut input = AgentTurnInput::from("first");
    input.turn = TurnExtensions::default().with(Arc::new(Note("unrendered note")));
    service.start_turn(session.id, input).await.unwrap();

    let requests = model.requests();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0]
        .input
        .iter()
        .any(|item| matches!(item, ModelInputItem::ContextFragment { .. })));
}

#[tokio::test]
async fn ambiguous_plain_name_does_not_select_but_exact_locator_does() {
    let model = RecordingAssistantModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let mut catalog = selectable_skills_fixture();
    let shadowed = selectable("system:workflow", "Workflow");
    catalog.skills.push(shadowed.clone());

    let mut ambiguous = AgentTurnInput::from("$workflow do it");
    ambiguous.turn = selecting(catalog.clone());
    service.start_turn(session.id, ambiguous).await.unwrap();
    assert!(!model.requests()[0].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ContextFragment { kind, .. } if kind == "skill_instructions"
    )));

    let mut exact = AgentTurnInput::from("use the exact package");
    exact.turn = selecting(catalog);
    exact.skills.push(AgentSkillRef {
        skill: skill_id(SkillScope::System, "workflow"),
    });
    service.start_turn(session.id, exact).await.unwrap();
    assert!(model.requests()[1].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ContextFragment {
            role: ModelContextRole::User,
            kind,
            content,
            ..
        } if kind == "skill_instructions" && content.contains(shadowed.id.as_str())
    )));
}

/// The names the selectable Skills' contributor excludes and environment
/// variables select nothing by a plain `$mention`; a linked package still
/// selects its Skill.
#[tokio::test]
async fn linked_package_has_priority_while_mention_exclusions_and_environment_variables_do_not_select(
) {
    let model = RecordingAssistantModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let catalog = selectable_skills_fixture();
    let skill = catalog.skills[0].id.clone();

    let mut blocked = AgentTurnInput::from("$workflow and $PATH");
    blocked.turn = selecting(SelectableSkills {
        mention_exclusions: vec!["WORKFLOW".to_string()],
        ..catalog.clone()
    });
    service.start_turn(session.id, blocked).await.unwrap();
    assert!(!model.requests()[0].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ContextFragment { kind, .. } if kind == "skill_instructions"
    )));

    let mut linked = AgentTurnInput::from(format!(
        "use [$different-label](skill://{skill}) while ignoring $PATH"
    ));
    linked.turn = selecting(SelectableSkills {
        mention_exclusions: vec!["different-label".to_string(), "workflow".to_string()],
        ..catalog
    });
    service.start_turn(session.id, linked).await.unwrap();
    assert!(model.requests()[1].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ContextFragment {
            role: ModelContextRole::User,
            kind,
            content,
            ..
        } if kind == "skill_instructions" && content.contains(skill.as_str())
    )));
}

/// The kernel's own tools are its knowledge: a plain `$mention` of one names
/// the tool, not a same-named Skill, though no contributor excluded it.
#[tokio::test]
async fn a_mention_of_a_kernel_tool_selects_no_skill_without_an_exclusion() {
    let model = RecordingAssistantModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let catalog = SelectableSkills {
        skills: vec![selectable("user:planner", "update_plan")],
        ..Default::default()
    };
    let skill_selected = |request: usize| {
        model.requests()[request].input.iter().any(|item| {
            matches!(
                item,
                ModelInputItem::ContextFragment {
                    kind,
                    content,
                    ..
                } if kind == "skill_instructions" && content.contains("user:planner")
            )
        })
    };

    let mut tool = AgentTurnInput::from("$Update_Plan first");
    tool.turn = selecting(catalog.clone());
    service.start_turn(session.id, tool).await.unwrap();
    assert!(!skill_selected(0));

    let mut folder = AgentTurnInput::from("$planner first");
    folder.turn = selecting(catalog);
    service.start_turn(session.id, folder).await.unwrap();
    assert!(skill_selected(1));
}

/// Whether a Skill is listed to the model is the `skills` plugin's decision;
/// the kernel commits any selectable Skill the user references.
#[tokio::test]
async fn an_explicitly_referenced_skill_is_committed_as_user_context() {
    let model = RecordingAssistantModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let skill = skill_id(SkillScope::User, "workflow");
    let mut input = AgentTurnInput::from("use the explicitly selected workflow");
    input.skills.push(AgentSkillRef {
        skill: skill.clone(),
    });
    input.turn = selecting(selectable_skills_fixture());
    service.start_turn(session.id, input).await.unwrap();

    let request = &model.requests()[0];
    assert!(request.input.iter().any(|item| matches!(
        item,
        ModelInputItem::ContextFragment {
            role: ModelContextRole::User,
            kind,
            content,
            ..
        } if kind == "skill_instructions" && content.contains(skill.as_str())
    )));
}

/// A Skill catalog that could not be built leaves the turn without Skills
/// (its contributor is left out), but a Skill the user referenced
/// explicitly is not silently dropped: that turn is refused.
#[tokio::test]
async fn an_explicit_skill_reference_without_a_catalog_still_fails_the_turn() {
    let model = RecordingAssistantModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let skill = skill_id(SkillScope::User, "workflow");
    let mut input = AgentTurnInput::from("use the explicitly selected workflow");
    input.skills.push(AgentSkillRef {
        skill: skill.clone(),
    });

    let refused = service.start_turn(session.id, input).await.unwrap_err();

    assert_eq!(
        refused,
        AgentError::InvalidInput(format!(
            "the Skill {skill} cannot be used: the Skill catalog is unavailable"
        ))
    );
    assert!(model.requests().is_empty());
    // Without a reference the same turn runs, without Skills.
    service
        .start_turn(session.id, AgentTurnInput::from("go on without Skills"))
        .await
        .unwrap();
}

/// The bug this contract exists to prevent.
///
/// A `SKILL.md` of eighteen kilobytes is an ordinary published Skill, and the
/// Skill catalog admits one. Turn admission used to re-derive its own, smaller
/// document budget and reject the result as `Persistence`, which failed the
/// whole conversation over a Skill that had loaded perfectly. `hanafubuki_agent_skills` owns
/// that bound; nothing here may second-guess it.
#[tokio::test]
async fn a_long_published_skill_document_is_admitted_rather_than_failing_the_turn() {
    let mut snapshot = selectable_skills_fixture();
    let body = "Guidance about how to use this Skill in practice.\n".repeat(400);
    snapshot.skills[0].context = format!(
        "<skill>\n<name>Workflow</name>\n---\nname: workflow\ndescription: Use the exact workflow\n---\n{body}\n</skill>"
    );
    assert!(
        snapshot.skills[0].context.len() > 12 * 1024,
        "the fixture must exceed the budget that used to reject it"
    );

    let expected = request(vec![
        skill_instructions_fragment("user:workflow", "user", &snapshot.skills[0].context),
        ModelInputItem::user_text("$workflow do it"),
    ]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("done"),
    )]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    service
        .start_turn(
            session.id,
            AgentTurnInput {
                text: "$workflow do it".to_string(),
                images: Vec::new(),
                skills: vec![AgentSkillRef {
                    skill: skill_id(SkillScope::User, "workflow"),
                }],
                turn: selecting(snapshot.clone()),
                world_state: Default::default(),
                permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::Manual,
                origin: Default::default(),
                provisional_title: None,
            },
        )
        .await
        .expect("a long Skill document must not fail turn admission");

    let committed = test_session_view(&service, session.id).unwrap();
    assert!(committed.model_context.iter().any(|item| matches!(
        item,
        ConversationItem::SkillContext { content, .. } if content.contains("in practice.")
    )));
}
