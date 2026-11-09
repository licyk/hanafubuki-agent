use super::*;

#[test]
fn catalog_order_and_revision_are_stable() {
    let provider = AgentToolProviderId::parse("provider".to_string()).unwrap();
    let left = AgentPlannedTool::fixed(
        provider.clone(),
        crate::AgentCoreTool::UpdatePlan.model_spec(),
        AgentToolExecutionClass::Exclusive,
    )
    .unwrap();
    let right = AgentPlannedTool::fixed(
        provider,
        crate::AgentCoreTool::NewContext.model_spec(),
        AgentToolExecutionClass::ConcurrentRead,
    )
    .unwrap();
    let first = AgentToolPlan::build(vec![left.clone(), right.clone()]).unwrap();
    let second = AgentToolPlan::build(vec![right, left]).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first
            .model_specs()
            .iter()
            .map(|tool| tool.name().unwrap().flattened_alias())
            .collect::<Vec<_>>(),
        vec!["new_context", "update_plan"]
    );
}

/// A schema reaches a protocol that takes JSON Schema whole (Responses,
/// Chat Completions) with its constraints, and is lowered to the portable
/// keywords for one that does not (Anthropic Messages); a kernel and a
/// sourced tool alike, never strict. The kernel's own copy stays whole.
#[tokio::test]
async fn schemas_are_lowered_only_for_a_protocol_that_cannot_take_them() {
    let schema = json!({
        "type": "object",
        "properties": {
            "kind": {"type": "string", "const": "demo"},
            "names": {
                "type": "array",
                "maxItems": 8,
                "uniqueItems": true,
                "items": {"type": "string", "minLength": 1, "pattern": "^[a-z]+$"}
            }
        },
        "required": ["kind", "names"],
        "additionalProperties": false
    });
    let mut model_spec = crate::AgentCoreTool::UpdatePlan.model_spec();
    model_spec.input_schema = schema.clone();
    let fixed = AgentPlannedTool::fixed(
        AgentToolProviderId::parse("builtin").unwrap(),
        model_spec,
        AgentToolExecutionClass::Exclusive,
    )
    .unwrap();
    let mut remote = dynamic_tool("remote", ToolExposure::Direct);
    remote.handler = Arc::new(TestHandler {
        descriptor: ToolDescriptor {
            input_schema: schema.clone(),
            ..descriptor(
                ToolName::parse("remote").unwrap(),
                "remote description",
                ToolExposure::Direct,
            )
        },
    });
    let sourced = plan_sourced(remote, ToolAvailability::Function);
    let deferred = dynamic_registration("deferred", ToolExposure::Deferred);
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![fixed, sourced, deferred],
    })]);
    for protocol in [
        hanafubuki_agent_model_api::ModelApiProtocol::Responses,
        hanafubuki_agent_model_api::ModelApiProtocol::ChatCompletions,
        hanafubuki_agent_model_api::ModelApiProtocol::AnthropicMessages,
    ] {
        let mut context = plan_context(AgentSessionMode::Default, false);
        context.provider_capabilities = protocol.tool_capabilities();
        let plan = planner
            .build(context, CancellationToken::default())
            .await
            .unwrap();
        let search = plan
            .registrations()
            .iter()
            .find(|tool| matches!(tool.binding, AgentPlannedToolBinding::PlanSearch))
            .unwrap();
        let ToolSpec::Function(search) = &search.provider_spec else {
            panic!("expected the plan's search function");
        };
        assert_eq!(
            search.input_schema["properties"]["query"]
                .get("maxLength")
                .is_some(),
            protocol.tool_capabilities().strict_json_schema,
            "{protocol:?}"
        );
        for name in ["update_plan", "remote"] {
            let tool = plan.resolve(None, name).unwrap();
            assert_eq!(tool.model_spec.input_schema, schema, "{name}");
            let ToolSpec::Function(provider) = &tool.provider_spec else {
                panic!("expected a function spec");
            };
            assert!(!provider.strict);
            if protocol.tool_capabilities().strict_json_schema {
                assert_eq!(provider.input_schema, schema, "{protocol:?} {name}");
                continue;
            }
            let names = &provider.input_schema["properties"]["names"];
            assert!(names.get("uniqueItems").is_none());
            assert!(names.get("maxItems").is_none());
            assert!(names["items"].get("pattern").is_none());
            assert!(names["items"].get("minLength").is_none());
            let kind = &provider.input_schema["properties"]["kind"];
            assert_eq!(kind["enum"], json!(["demo"]));
            assert!(kind.get("const").is_none());
        }
    }
}

/// Wire names are unique: of two tools named alike, the later one in
/// planning order (here by provider id) is renamed with a hash of its
/// identity, the same on every plan, and the plan does not fail.
#[tokio::test]
async fn a_name_collision_renames_the_later_tool_deterministically() {
    let tool = |provider: &str| {
        AgentPlannedTool::fixed(
            AgentToolProviderId::parse(provider.to_string()).unwrap(),
            crate::AgentCoreTool::NewContext.model_spec(),
            AgentToolExecutionClass::Exclusive,
        )
        .unwrap()
    };
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![tool("second"), tool("first")],
    })]);
    let wire_names = |plan: &AgentToolPlan| {
        plan.registrations()
            .iter()
            .map(|tool| {
                (
                    tool.provider_id.as_str().to_string(),
                    tool.provider_spec.name().unwrap().local_name().to_string(),
                )
            })
            .collect::<Vec<_>>()
    };
    let plan = planner
        .build(
            plan_context(AgentSessionMode::Default, true),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    let names = wire_names(&plan);
    assert_eq!(names[0], ("first".to_string(), "new_context".to_string()));
    assert_eq!(names[1].0, "second");
    assert!(names[1].1.starts_with("new_context_"));
    assert_eq!(
        plan.resolve(None, &names[1].1)
            .unwrap()
            .provider_id
            .as_str(),
        "second"
    );
    assert_eq!(plan.omissions().len(), 1);
    let again = planner
        .build(
            plan_context(AgentSessionMode::Default, true),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(wire_names(&again), names);
}

#[test]
fn core_tools_are_direct_exclusive_and_name_their_actions() {
    let registrations = core_tool_registrations(false).unwrap();
    let catalog = AgentToolPlan::build(registrations.clone()).unwrap();
    for registration in &registrations {
        assert_eq!(
            registration.execution_class,
            AgentToolExecutionClass::Exclusive,
            "{}",
            registration.model_spec.name
        );
        assert_eq!(registration.exposure, ToolExposure::Direct);
    }
    for (name, action) in [
        ("new_context", AgentActivityAction::USE_TOOL),
        ("update_plan", AgentActivityAction::UPDATE_PLAN),
        (
            "request_user_input",
            AgentActivityAction::REQUEST_USER_INPUT,
        ),
    ] {
        assert_eq!(
            catalog.resolve(None, name).unwrap().presentation.action,
            action
        );
    }
}

#[tokio::test]
async fn fresh_default_plan_advertises_direct_tools_and_defers_the_rest_without_tool_search() {
    let planner = AgentToolPlanner::new(vec![
        Arc::new(CoreAgentToolContributor),
        Arc::new(DynamicContributor {
            registrations: vec![dynamic_registration(
                "deferred_mutation",
                ToolExposure::Deferred,
            )],
        }),
    ]);

    for native_tool_search in [false, true] {
        let mut context = plan_context(AgentSessionMode::Default, true);
        context.provider_capabilities.native_tool_search = native_tool_search;
        let plan = planner
            .build(context, CancellationToken::default())
            .await
            .unwrap();

        assert!(plan.resolve(None, "update_plan").is_ok());
        assert!(plan.model_specs().iter().any(|specification| {
            matches!(
                specification,
                ToolSpec::Function(function)
                    if function.name.local_name() == "update_plan"
            )
        }));
        assert!(plan.resolve(None, "deferred_mutation").is_err());
        let deferred_call = hanafubuki_agent_model_api::ModelToolCall {
            item_id: "deferred-item".to_string(),
            call_id: "deferred-call".to_string(),
            namespace: None,
            name: "deferred_mutation".to_string(),
            arguments: json!({}),
            argument_error: None,
        };
        assert_eq!(
            plan.unselected_deferred_call(&deferred_call)
                .map(|registration| registration.model_spec.name.as_str()),
            Some("deferred_mutation")
        );
        let unknown_call = hanafubuki_agent_model_api::ModelToolCall {
            name: "unknown_mutation".to_string(),
            ..deferred_call
        };
        assert!(plan.unselected_deferred_call(&unknown_call).is_none());
    }
}
