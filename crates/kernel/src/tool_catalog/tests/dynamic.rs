use super::*;

#[tokio::test]
async fn planner_freezes_visibility_and_deferred_index_from_one_contribution() {
    let planner = AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![
            dynamic_registration("direct", ToolExposure::Direct),
            dynamic_registration("deferred", ToolExposure::Deferred),
            dynamic_registration("hidden", ToolExposure::Hidden),
            dynamic_registration("model_only", ToolExposure::DirectModelOnly),
        ],
    })]);
    let plan = planner
        .build(
            plan_context(AgentSessionMode::Default, false),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        plan.model_specs()
            .into_iter()
            .map(|tool| match tool {
                ToolSpec::Function(tool) => (tool.name.flattened_alias(), tool.strict),
                _ => panic!("expected a function tool"),
            })
            .collect::<Vec<_>>(),
        vec![
            ("direct".into(), false),
            ("model_only".into(), false),
            ("tool_search".into(), false),
        ]
    );
    assert_eq!(
        plan.deferred_registrations()
            .map(|tool| tool.model_spec.name.as_str())
            .collect::<Vec<_>>(),
        vec!["deferred"]
    );
    assert!(plan.resolve(None, "hidden").is_err());
    assert!(plan.resolve(None, "model_only").is_ok());
}

/// A schema nested past the depth bound.
fn too_deep_schema() -> serde_json::Value {
    let mut schema = json!({"type": "string"});
    for _ in 0..=hanafubuki_agent_tool_contracts::MAX_TOOL_SCHEMA_DEPTH {
        schema = json!({"type": "object", "properties": {"inner": schema}});
    }
    schema
}

fn sourced_with_schema(generation: u8, schema: serde_json::Value) -> AgentPlannedTool {
    let mut tool = descriptor(
        ToolName::parse("remote").unwrap(),
        "remote description",
        ToolExposure::Direct,
    );
    tool.input_schema = schema;
    plan_sourced(
        sourced_tool("dynamic", "remote", [generation; 32], tool),
        ToolAvailability::Function,
    )
}

async fn plan_with(
    cache: &AgentToolSpecCache,
    tool: AgentPlannedTool,
) -> Result<AgentToolPlan, AgentToolCatalogError> {
    AgentToolPlanner::new(vec![Arc::new(DynamicContributor {
        registrations: vec![tool],
    })])
    .with_spec_cache(cache.clone())
    .build(
        plan_context(AgentSessionMode::Default, true),
        CancellationToken::default(),
    )
    .await
}

/// A tool's specs are validated once per identity, not on every step: the
/// cache keeps what passed, and a new generation (a new catalog) is
/// validated again.
#[tokio::test]
async fn a_tools_schema_is_validated_once_per_identity() {
    let cache = AgentToolSpecCache::default();
    let valid = json!({"type": "object", "properties": {}, "additionalProperties": false});

    let first = plan_with(&cache, sourced_with_schema(1, valid.clone()))
        .await
        .unwrap();
    assert_eq!(first.registrations().len(), 1);
    assert_eq!(cache.len(), 1);
    plan_with(&cache, sourced_with_schema(1, valid))
        .await
        .unwrap();
    assert_eq!(cache.len(), 1, "the same identity is not validated again");

    // A new generation is a new catalog, validated before it is planned: an
    // optional tool whose schema fails is omitted, not cached.
    let rejected = plan_with(&cache, sourced_with_schema(2, too_deep_schema()))
        .await
        .unwrap();
    assert!(rejected.registrations().is_empty());
    assert!(rejected.omissions()[0].reason.contains("invalid schema"));
    assert_eq!(cache.len(), 1);
}
