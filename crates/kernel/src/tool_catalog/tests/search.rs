use hanafubuki_agent_tool_contracts::{FreeformToolFormat, FreeformToolSpec};

use super::*;

fn namespaced_search_registration(local_name: &str) -> AgentPlannedTool {
    let mut tool = sourced_tool(
        "mcp.calendar",
        &format!("remote-{local_name}"),
        [local_name.len() as u8; 32],
        ToolDescriptor {
            input_schema: json!({
                "type": "object",
                "properties": {
                    "attendee_email": {
                        "type": "string",
                        "description": "RFC mailbox recipient"
                    }
                },
                "additionalProperties": false
            }),
            execution_class: AgentToolExecutionClass::Exclusive,
            effect: ToolEffect::Mutating,
            ..descriptor(
                ToolName::namespaced(vec!["mcp".to_string(), "calendar".to_string()], local_name)
                    .unwrap(),
                "Schedule corporate events",
                ToolExposure::Deferred,
            )
        },
    );
    tool.namespace_description = Some("Corporate agenda server instructions".into());
    tool.subject = Some("calendar MCP".into());
    plan_sourced(tool, ToolAvailability::Function)
}

fn relevance_fixture_registration(
    provider: &str,
    name: &str,
    description: &str,
) -> AgentPlannedTool {
    plan_sourced(
        sourced_tool(
            provider,
            &name.replace('_', "-"),
            [name.len() as u8; 32],
            descriptor(
                ToolName::parse(name).unwrap(),
                description,
                ToolExposure::Deferred,
            ),
        ),
        ToolAvailability::Function,
    )
}

#[tokio::test]
async fn mode_and_provider_capabilities_filter_sourced_tools_before_search_indexing() {
    let mut tool = dynamic_tool("remote_mutation", ToolExposure::Deferred);
    tool.handler = Arc::new(TestHandler {
        descriptor: ToolDescriptor {
            execution_class: AgentToolExecutionClass::Exclusive,
            effect: ToolEffect::Mutating,
            ..tool.handler.descriptor().clone()
        },
    });
    let mutation = plan_sourced(tool, ToolAvailability::Function);
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![mutation],
    })]);

    let plan_mode = planner
        .build(
            plan_context(AgentSessionMode::Plan, true),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert!(plan_mode.deferred_registrations().next().is_none());
    assert!(plan_mode.search_deferred("mutation", 10).is_empty());

    let mut unsupported_context = plan_context(AgentSessionMode::Default, true);
    unsupported_context.provider_capabilities.function_tools = false;
    let unsupported = planner
        .build(unsupported_context, CancellationToken::default())
        .await
        .unwrap();
    assert!(unsupported.registrations().is_empty());
}

#[tokio::test]
async fn fallback_search_is_stable_and_exact_selection_promotes_next_plan() {
    let calendar = dynamic_registration("calendar_create", ToolExposure::Deferred);
    let weather = dynamic_registration("weather_lookup", ToolExposure::Deferred);
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![weather.clone(), calendar.clone()],
    })]);
    let first = planner
        .build(
            plan_context(AgentSessionMode::Default, true),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert!(first.model_specs().iter().any(|specification| {
        matches!(specification, ToolSpec::Function(function)
            if function.name.local_name() == hanafubuki_agent_model_api::FALLBACK_TOOL_SEARCH_NAME)
    }));
    assert!(!first.model_specs().iter().any(|specification| {
        matches!(specification, ToolSpec::Function(function)
            if function.name.local_name() == "calendar_create")
    }));
    let calendar_call = hanafubuki_agent_model_api::ModelToolCall {
        item_id: "calendar-item".into(),
        call_id: "calendar-call".into(),
        namespace: None,
        name: "calendar_create".into(),
        arguments: json!({}),
        argument_error: None,
    };
    assert!(first.resolve_call(&calendar_call).is_err());
    assert!(first.unselected_deferred_call(&calendar_call).is_some());
    let matches = first.search_deferred("calendar", 10);
    assert_eq!(
        matches
            .iter()
            .map(|entry| entry.model_name.as_str())
            .collect::<Vec<_>>(),
        vec!["calendar_create"]
    );
    let mut next_context = plan_context(AgentSessionMode::Default, true);
    next_context
        .selected_deferred_tools
        .insert(matches[0].selection.clone());
    let next = planner
        .build(next_context, CancellationToken::default())
        .await
        .unwrap();
    assert!(next.resolve(None, "calendar_create").is_ok());
    assert!(next.resolve_call(&calendar_call).is_ok());
    assert!(next.resolve(None, "weather_lookup").is_err());
    assert!(next.model_specs().iter().any(|specification| {
        matches!(specification, ToolSpec::Function(function)
            if function.name.local_name() == "calendar_create")
    }));

    let stale = AgentToolSelection {
        generation: AgentToolGeneration([99; 32]),
        ..AgentToolSelection::from_planned_tool(&calendar)
    };
    let mut stale_context = plan_context(AgentSessionMode::Default, true);
    stale_context.selected_deferred_tools.insert(stale);
    let stale_plan = planner
        .build(stale_context, CancellationToken::default())
        .await
        .unwrap();
    assert!(stale_plan.resolve(None, "calendar_create").is_err());
    assert!(stale_plan
        .unselected_deferred_call(&calendar_call)
        .is_some());
}

#[tokio::test]
async fn native_tool_search_hides_deferred_specs_until_exact_search_selection() {
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![dynamic_registration(
            "calendar_create",
            ToolExposure::Deferred,
        )],
    })]);
    let mut context = plan_context(AgentSessionMode::Default, true);
    context.provider_capabilities.native_tool_search = true;
    let plan = planner
        .build(context.clone(), CancellationToken::default())
        .await
        .unwrap();
    assert!(plan.native_tool_search());
    assert!(plan
        .model_specs()
        .iter()
        .any(|specification| { matches!(specification, ToolSpec::ToolSearch(_)) }));
    assert!(!plan.model_specs().iter().any(|specification| {
        matches!(specification, ToolSpec::Function(function) if function.name.local_name() == "calendar_create")
    }));
    assert!(plan.resolve(None, "calendar_create").is_err());
    let calendar_call = hanafubuki_agent_model_api::ModelToolCall {
        item_id: "calendar-item".into(),
        call_id: "calendar-call".into(),
        namespace: None,
        name: "calendar_create".into(),
        arguments: json!({}),
        argument_error: None,
    };
    assert!(plan.unselected_deferred_call(&calendar_call).is_some());
    assert!(plan
        .resolve_call(&hanafubuki_agent_model_api::ModelToolCall {
            item_id: "search-1".into(),
            call_id: "search-1".into(),
            namespace: None,
            name: hanafubuki_agent_model_api::NATIVE_TOOL_SEARCH_NAME.into(),
            arguments: json!({"query": "calendar"}),
            argument_error: None,
        })
        .is_ok());
    assert!(plan
        .resolve(None, hanafubuki_agent_model_api::FALLBACK_TOOL_SEARCH_NAME)
        .is_err());
    let matched = plan.search_deferred("calendar", 8);
    assert_eq!(matched.len(), 1);
    context
        .selected_deferred_tools
        .insert(matched[0].selection.clone());
    let selected_plan = planner
        .build(context, CancellationToken::default())
        .await
        .unwrap();
    assert!(selected_plan.resolve(None, "calendar_create").is_ok());
    assert!(selected_plan.resolve_call(&calendar_call).is_ok());
    assert!(!selected_plan.model_specs().iter().any(|specification| {
        matches!(specification, ToolSpec::Function(function) if function.name.local_name() == "calendar_create")
    }));
}

#[tokio::test]
async fn bm25_indexes_namespaces_instructions_schema_fields_and_stable_sources() {
    let create = namespaced_search_registration("create_event");
    let delete = namespaced_search_registration("delete_event");
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![delete, create],
    })]);
    let mut context = plan_context(AgentSessionMode::Default, true);
    context.provider_capabilities.native_tool_search = true;
    context.provider_capabilities.namespace_tools = true;
    let plan = planner
        .build(context, CancellationToken::default())
        .await
        .unwrap();

    for query in [
        "create event",
        "corporate agenda",
        "attendee email",
        "mailbox recipient",
        "mcp calendar",
    ] {
        assert!(
            plan.search_deferred(query, DEFAULT_AGENT_TOOL_SEARCH_RESULTS)
                .iter()
                .any(|matched| matched.model_name == "mcp__calendar__create_event"),
            "query {query:?} should discover create_event"
        );
    }
    let coalesced = coalesce_search_specs(
        plan.search_deferred("calendar", DEFAULT_AGENT_TOOL_SEARCH_RESULTS)
            .into_iter()
            .map(|matched| matched.specification),
    );
    let [ToolSpec::Namespace(namespace)] = coalesced.as_slice() else {
        panic!("namespace matches should be merged into one output");
    };
    assert_eq!(namespace.name.local_name(), "mcp__calendar");
    assert_eq!(
        namespace
            .tools
            .iter()
            .map(|tool| tool.name.local_name())
            .collect::<Vec<_>>(),
        vec!["create_event", "delete_event"]
    );
}

#[tokio::test]
async fn offline_relevance_fixture_meets_recall_mrr_ndcg_and_cjk_baselines() {
    let tools = vec![
        relevance_fixture_registration(
            "builtin.task-center",
            "inspect_task_logs",
            "Read bounded Task Center terminal and application logs with a cursor",
        ),
        relevance_fixture_registration(
            "builtin.task-center",
            "list_tasks",
            "List current Task Center jobs and their lifecycle status",
        ),
        relevance_fixture_registration(
            "skills.catalog",
            "skills_read",
            "Read a paged UTF-8 Skill package resource and its instructions",
        ),
        relevance_fixture_registration(
            "mcp.chinese-operations",
            "mcp_task_log_lookup",
            "查询任务中心日志和终端输出，支持游标分页",
        ),
        relevance_fixture_registration(
            "builtin.web",
            "fetch_web_page",
            "Fetch bounded readable content from one public web page",
        ),
        relevance_fixture_registration("fixture.tie", "tie_alpha", "stable ranking fixture"),
        relevance_fixture_registration("fixture.tie", "tie_beta", "stable ranking fixture"),
    ];
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: tools,
    })]);
    let plan = planner
        .build(
            plan_context(AgentSessionMode::Default, true),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    let cases = [
        ("task terminal logs", &["inspect_task_logs"][..]),
        ("current jobs lifecycle", &["list_tasks"][..]),
        ("paged utf-8 skill instructions", &["skills_read"][..]),
        ("查询任务中心日志终端输出", &["mcp_task_log_lookup"][..]),
        ("public web page readable", &["fetch_web_page"][..]),
    ];
    let mut recall_at_8 = 0.0;
    let mut reciprocal_rank = 0.0;
    let mut normalized_dcg = 0.0;
    for (query, relevant) in cases {
        let ranked = plan.search_deferred(query, 8);
        let relevant_found = ranked
            .iter()
            .filter(|matched| relevant.contains(&matched.model_name.as_str()))
            .count();
        recall_at_8 += relevant_found as f64 / relevant.len() as f64;
        let first_rank = ranked
            .iter()
            .position(|matched| relevant.contains(&matched.model_name.as_str()))
            .map(|index| index + 1);
        reciprocal_rank += first_rank.map_or(0.0, |rank| 1.0 / rank as f64);
        let dcg = ranked
            .iter()
            .enumerate()
            .filter(|(_, matched)| relevant.contains(&matched.model_name.as_str()))
            .map(|(index, _)| 1.0 / ((index + 2) as f64).log2())
            .sum::<f64>();
        let ideal = (0..relevant.len())
            .map(|index| 1.0 / ((index + 2) as f64).log2())
            .sum::<f64>();
        normalized_dcg += dcg / ideal;
    }
    let count = cases.len() as f64;
    assert!(recall_at_8 / count >= 1.0);
    assert!(reciprocal_rank / count >= 1.0);
    assert!(normalized_dcg / count >= 1.0);

    let ties = plan.search_deferred("stable ranking fixture", 8);
    let tied_names = ties
        .iter()
        .filter(|matched| matched.model_name.starts_with("tie_"))
        .map(|matched| matched.model_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(tied_names, vec!["tie_alpha", "tie_beta"]);
    assert_eq!(
        tied_names,
        plan.search_deferred("stable ranking fixture", 8)
            .iter()
            .filter(|matched| matched.model_name.starts_with("tie_"))
            .map(|matched| matched.model_name.as_str())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn bm25_indexes_freeform_syntax_and_reuses_only_identical_frozen_indexes() {
    let freeform_tool = sourced_tool(
        "renderer",
        "render-dsl",
        [7; 32],
        descriptor(
            ToolName::parse("render_dsl").unwrap(),
            "render_dsl description",
            ToolExposure::Deferred,
        ),
    );
    let freeform_availability = ToolAvailability::Freeform(FreeformToolSpec {
        name: ToolName::parse("render_dsl").unwrap(),
        description: "Render a scene".into(),
        format: FreeformToolFormat {
            format_type: "grammar".into(),
            syntax: "lark scene_node material_ref".into(),
            definition: "start: scene_node".into(),
        },
    });
    let freeform = plan_sourced(freeform_tool.clone(), freeform_availability.clone());
    let cache = AgentToolSearchIndexCache::default();
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![freeform.clone()],
    })])
    .with_search_cache(cache.clone());
    let mut context = plan_context(AgentSessionMode::Default, true);
    context.provider_capabilities.freeform_tools = true;
    let first = planner
        .build(context.clone(), CancellationToken::default())
        .await
        .unwrap();
    let second = planner
        .build(context.clone(), CancellationToken::default())
        .await
        .unwrap();
    assert!(first.shares_search_index_with(&second));
    assert_eq!(first.search_deferred("material ref", 8).len(), 1);

    let changed = plan_sourced(
        SourcedTool {
            namespace_description: Some("new source generation".into()),
            ..freeform_tool
        },
        freeform_availability,
    );
    let changed_plan = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![changed],
    })])
    .with_search_cache(cache.clone())
    .build(context, CancellationToken::default())
    .await
    .unwrap();
    assert!(!first.shares_search_index_with(&changed_plan));
    let restored = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![freeform],
    })])
    .with_search_cache(cache)
    .build(
        {
            let mut restored_context = plan_context(AgentSessionMode::Default, true);
            restored_context.provider_capabilities.freeform_tools = true;
            restored_context
        },
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert!(first.shares_search_index_with(&restored));
}

/// A sourced tool named like a core tool or `tool_search` keeps its place
/// under a renamed wire name, whatever its trust, and the rename is
/// reported; only an invalid tool is omitted.
#[tokio::test]
async fn sourced_collisions_are_renamed_and_invalid_tools_omitted() {
    let conflicting = dynamic_registration("update_plan", ToolExposure::Direct);
    let mut invalid = dynamic_registration("optional_invalid", ToolExposure::Direct);
    invalid.model_spec.input_schema = json!(null);
    let reserved = dynamic_registration("tool_search", ToolExposure::Direct);
    let required = plan_sourced(
        SourcedTool {
            trust: ToolSourceTrust::Required,
            ..dynamic_tool("new_context", ToolExposure::Direct)
        },
        ToolAvailability::Function,
    );
    let plan = AgentToolPlanner::new(vec![
        Arc::new(CoreAgentToolContributor),
        Arc::new(DynamicContributor {
            registrations: vec![conflicting, invalid, reserved, required],
        }),
    ])
    .build(
        plan_context(AgentSessionMode::Default, true),
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert!(matches!(
        plan.resolve(None, "update_plan").unwrap().binding,
        AgentPlannedToolBinding::Core(crate::AgentCoreTool::UpdatePlan)
    ));
    assert!(matches!(
        plan.resolve(None, "new_context").unwrap().binding,
        AgentPlannedToolBinding::Core(crate::AgentCoreTool::NewContext)
    ));
    let renamed = |name: &str| {
        plan.registrations()
            .iter()
            .find(|tool| tool.model_spec.name == name && tool.is_sourced())
            .and_then(|tool| tool.provider_spec.name())
            .map(|name| name.local_name().to_string())
            .unwrap()
    };
    for name in ["update_plan", "tool_search", "new_context"] {
        let wire = renamed(name);
        assert!(wire.starts_with(&format!("{name}_")), "{wire}");
        assert_eq!(wire.len(), name.len() + 13);
        assert!(plan.resolve(None, &wire).is_ok());
    }
    let reasons = plan
        .omissions()
        .iter()
        .map(|omission| omission.reason.as_str())
        .collect::<Vec<_>>();
    assert_eq!(reasons.len(), 4, "{reasons:?}");
    assert_eq!(
        reasons
            .iter()
            .filter(|reason| reason.starts_with("renamed to `"))
            .count(),
        3
    );
    assert!(reasons
        .iter()
        .any(|reason| reason.starts_with("invalid tool")));
}

#[tokio::test]
async fn native_search_is_added_after_fair_budget_defers_direct_external_tools() {
    let registrations = (0..(MAX_AGENT_DIRECT_TOOL_ENTRIES + 1))
        .map(|index| dynamic_registration(&format!("budget_{index:03}"), ToolExposure::Direct))
        .collect();
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor { registrations })]);
    let mut context = plan_context(AgentSessionMode::Default, true);
    context.provider_capabilities.native_tool_search = true;
    let plan = planner
        .build(context, CancellationToken::default())
        .await
        .unwrap();
    assert!(plan
        .model_specs()
        .iter()
        .any(|specification| matches!(specification, ToolSpec::ToolSearch(_))));
    assert_eq!(plan.deferred_registrations().count(), 1);
}

#[tokio::test]
async fn oversized_sourced_catalog_is_deferred_by_stable_name_order() {
    let registrations = (0..(MAX_AGENT_DIRECT_TOOL_ENTRIES + 12))
        .rev()
        .map(|index| dynamic_registration(&format!("dynamic_{index:03}"), ToolExposure::Direct))
        .collect::<Vec<_>>();
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor { registrations })]);
    let plan = planner
        .build(
            plan_context(AgentSessionMode::Default, true),
            CancellationToken::default(),
        )
        .await
        .unwrap();

    let direct = plan
        .registrations()
        .iter()
        .filter(|registration| registration.exposure == ToolExposure::Direct)
        .map(|registration| registration.model_spec.name.clone())
        .collect::<Vec<_>>();
    let deferred = plan
        .deferred_registrations()
        .map(|registration| registration.model_spec.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(direct.len(), MAX_AGENT_DIRECT_TOOL_ENTRIES + 1);
    assert_eq!(direct.first().map(String::as_str), Some("dynamic_000"));
    assert_eq!(direct[MAX_AGENT_DIRECT_TOOL_ENTRIES - 1], "dynamic_127");
    assert_eq!(direct.last().map(String::as_str), Some("tool_search"));
    assert_eq!(
        deferred,
        (MAX_AGENT_DIRECT_TOOL_ENTRIES..MAX_AGENT_DIRECT_TOOL_ENTRIES + 12)
            .map(|index| format!("dynamic_{index:03}"))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn oversized_sourced_catalog_allocates_direct_slots_fairly_across_providers() {
    let mut registrations = Vec::new();
    for index in 0..MAX_AGENT_DIRECT_TOOL_ENTRIES {
        for (provider, prefix) in [("alpha", "a"), ("beta", "b")] {
            let mut registration =
                dynamic_registration(&format!("{prefix}_{index:03}"), ToolExposure::Direct);
            registration.provider_id = AgentToolProviderId::parse(provider).unwrap();
            registrations.push(registration);
        }
    }
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor { registrations })]);
    let plan = planner
        .build(
            plan_context(AgentSessionMode::Default, true),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    let direct_for = |provider: &str| {
        plan.registrations()
            .iter()
            .filter(|registration| {
                registration.exposure == ToolExposure::Direct
                    && registration.provider_id.as_str() == provider
            })
            .count()
    };
    let alpha = direct_for("alpha");
    let beta = direct_for("beta");
    assert!(alpha > 0 && beta > 0);
    assert!(alpha.abs_diff(beta) <= 1, "alpha={alpha}, beta={beta}");
}

#[test]
fn sourced_arguments_are_validated_against_the_frozen_schema() {
    let schema = json!({
        "type": "object",
        "properties": {"name": {"type": "string", "maxLength": 4}},
        "required": ["name"],
        "additionalProperties": false
    });
    validate_dynamic_tool_arguments(&schema, &json!({"name": "okay"})).unwrap();
    assert!(validate_dynamic_tool_arguments(&schema, &json!({"name": "too-long"})).is_err());
    assert!(validate_dynamic_tool_arguments(&schema, &json!({"other": true})).is_err());
}
