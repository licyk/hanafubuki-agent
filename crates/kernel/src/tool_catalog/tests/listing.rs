use super::*;

#[tokio::test]
async fn listing_keeps_fitted_native_namespaces_distinct() {
    let tools = ["one", "two"]
        .into_iter()
        .map(|provider| {
            plan_sourced(
                sourced_tool(
                    provider,
                    "same-id",
                    [1; 32],
                    descriptor(
                        ToolName::namespaced(vec!["namespace".into()], "long".repeat(30)).unwrap(),
                        "Collision fixture",
                        ToolExposure::Deferred,
                    ),
                ),
                ToolAvailability::Function,
            )
        })
        .collect();
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: tools,
    })]);
    let mut context = plan_context(AgentSessionMode::Default, false);
    context.provider_capabilities =
        hanafubuki_agent_model_api::ModelApiProtocol::Responses.tool_capabilities();
    let plan = planner
        .build(context, CancellationToken::default())
        .await
        .unwrap();
    let page = plan.list_deferred(None, 0, 50);
    assert_eq!(page.tools.len(), 2);
    assert_ne!(page.tools[0], page.tools[1]);
    for name in page.tools {
        assert!(name.len() <= 128);
        let found = plan.search_deferred(&name, 1);
        assert_eq!(found[0].model_name, name);
        assert_eq!(plan.search_deferred(&format!("  {name}  "), 20).len(), 1);
        let wire =
            crate::tool_catalog::provider_table::ProviderToolName::of_spec(&found[0].specification)
                .unwrap();
        assert_eq!(
            crate::flattened_tool_call_name(wire.namespace.as_deref(), &wire.name),
            name
        );
    }
}

#[tokio::test]
async fn list_pages_contain_only_deferred_names_without_admitting_them() {
    for protocol in [
        hanafubuki_agent_model_api::ModelApiProtocol::Responses,
        hanafubuki_agent_model_api::ModelApiProtocol::ChatCompletions,
        hanafubuki_agent_model_api::ModelApiProtocol::AnthropicMessages,
    ] {
        let mut registrations = (0..207)
            .rev()
            .map(|index| dynamic_registration(&format!("tool_{index:03}"), ToolExposure::Deferred))
            .collect::<Vec<_>>();
        registrations.push(dynamic_registration("direct", ToolExposure::Direct));
        registrations.push(dynamic_registration("hidden", ToolExposure::Hidden));
        registrations.push(plan_sourced(
            sourced_tool(
                "other-source",
                "other-tool",
                [3; 32],
                descriptor(
                    ToolName::parse("other").unwrap(),
                    "Other source",
                    ToolExposure::Deferred,
                ),
            ),
            ToolAvailability::Function,
        ));
        let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor { registrations })]);
        let mut context = plan_context(AgentSessionMode::Default, false);
        context.provider_capabilities = protocol.tool_capabilities();
        let plan = planner
            .build(context, CancellationToken::default())
            .await
            .unwrap();
        let listing = plan.resolve(None, "tool_list").unwrap();
        assert_eq!(listing.binding, AgentPlannedToolBinding::PlanList);
        assert_eq!(
            listing.execution_class,
            AgentToolExecutionClass::ConcurrentRead
        );
        assert!(matches!(listing.provider_spec, ToolSpec::Function(_)));
        assert!(plan.resolve(None, "tool_000").is_err());
        let mut names = Vec::new();
        let mut offset = 0;
        loop {
            let page = plan.list_deferred(Some("dynamic"), offset, MAX_AGENT_TOOL_LIST_RESULTS);
            assert_eq!(page.total, 207);
            assert!(page.tools.len() <= 50);
            names.extend(page.tools);
            match page.next_offset {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert_eq!(
            names,
            (0..207)
                .map(|index| format!("tool_{index:03}"))
                .collect::<Vec<_>>()
        );
        assert_eq!(plan.list_deferred(None, 0, 50).total, 208);
        for page in [
            plan.list_deferred(Some("unknown"), 0, 50),
            plan.list_deferred(None, usize::MAX, 50),
        ] {
            assert!(page.tools.is_empty());
            assert_eq!(page.next_offset, None);
        }
        assert!(plan.resolve(None, "tool_000").is_err());
        assert_eq!(
            plan.search_deferred("tool_000", 1)[0].model_name,
            "tool_000"
        );
    }
}

#[tokio::test]
async fn listing_is_absent_without_deferred_tools_and_reserves_its_name() {
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![dynamic_registration("direct", ToolExposure::Direct)],
    })]);
    let plan = planner
        .build(
            plan_context(AgentSessionMode::Default, false),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert!(plan.resolve(None, "tool_list").is_err());
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![dynamic_registration("tool_list", ToolExposure::Deferred)],
    })]);
    let plan = planner
        .build(
            plan_context(AgentSessionMode::Default, false),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    let page = plan.list_deferred(None, 0, 50);
    assert_eq!(page.tools.len(), 1);
    assert!(page.tools[0].starts_with("tool_list_"));
    assert_eq!(
        plan.resolve(None, "tool_list").unwrap().binding,
        AgentPlannedToolBinding::PlanList
    );
    assert_eq!(
        plan.search_deferred(&page.tools[0], 1)[0].model_name,
        page.tools[0]
    );
}
