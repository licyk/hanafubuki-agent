use super::*;

/// A catalog built from a listing in which every tool decoded.
fn build_tool_catalog(
    config: &McpServerConfig,
    connection_generation: McpConnectionGeneration,
    tools: Vec<Tool>,
) -> Result<McpToolCatalog, McpError> {
    crate::catalog::build_tool_catalog(
        config,
        connection_generation,
        crate::listing::McpListing {
            items: tools,
            undecodable: Vec::new(),
        },
    )
}

#[test]
fn discovery_sanitizes_remote_names_per_character() {
    let schema = Map::from_iter(
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let tool = Tool::new("remote/tool", "Remote read", schema.clone())
        .with_annotations(ToolAnnotations::new().read_only(true).destructive(false));
    let McpToolCatalog {
        generation,
        tools: bindings,
        ..
    } = build_tool_catalog(
        &stdio_config(),
        McpConnectionGeneration([7; 32]),
        vec![tool],
    )
    .unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].catalog_generation, generation);
    assert_eq!(
        bindings[0].specification.name.qualified(),
        "mcp::local::remote_tool"
    );
    assert_eq!(bindings[0].remote_name, "remote/tool");
    assert!(bindings[0].hints.read_only);
    assert_eq!(bindings[0].exposure, McpExposurePolicy::Deferred);
    assert!(!bindings[0].specification.strict);

    let mut overridden = stdio_config();
    overridden
        .tool_approval_policies
        .insert("remote/tool".into(), McpApprovalPolicy::Deny);
    overridden
        .tool_exposure_policies
        .insert("remote/tool".into(), McpExposurePolicy::Direct);
    let overridden_bindings = build_tool_catalog(
        &overridden,
        McpConnectionGeneration([7; 32]),
        vec![Tool::new("remote/tool", "Remote read", schema)],
    )
    .unwrap()
    .tools;
    assert_eq!(overridden_bindings[0].exposure, McpExposurePolicy::Direct);
    assert_ne!(
        bindings[0].tool_generation,
        overridden_bindings[0].tool_generation
    );
    assert_eq!(
        overridden.approval_policy_for("remote/tool"),
        McpApprovalPolicy::Deny
    );
}

#[test]
fn discovery_filters_empty_catalogs_and_hard_limits_are_deterministic() {
    let schema = Map::from_iter(
        json!({"type": "object", "properties": {}, "additionalProperties": false})
            .as_object()
            .unwrap()
            .clone(),
    );
    let mut config = stdio_config();
    config.enabled_tools.insert("kept".into());
    let filtered = build_tool_catalog(
        &config,
        McpConnectionGeneration([1; 32]),
        vec![
            Tool::new("kept", "Kept", schema.clone()),
            Tool::new("removed", "Removed", schema.clone()),
        ],
    )
    .unwrap()
    .tools;
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].remote_name, "kept");

    let empty = build_tool_catalog(
        &stdio_config(),
        McpConnectionGeneration([2; 32]),
        Vec::new(),
    )
    .unwrap()
    .tools;
    assert!(empty.is_empty());
}

#[test]
fn mcp_apps_visibility_hides_non_model_tools_without_trusting_annotations() {
    let schema = Map::from_iter(
        json!({"type": "object", "properties": {}, "additionalProperties": false})
            .as_object()
            .unwrap()
            .clone(),
    );
    let mut app_only = Tool::new("app_only", "Only for the app UI", schema.clone());
    app_only.meta = Some(rmcp::model::MetaObject(Map::from_iter([(
        "ui".into(),
        json!({"visibility": ["app"]}),
    )])));
    let mut model_and_app = Tool::new("model_and_app", "Visible to both", schema);
    model_and_app.meta = Some(rmcp::model::MetaObject(Map::from_iter([(
        "ui".into(),
        json!({"visibility": ["app", "model"]}),
    )])));
    let mut model_only = Tool::new(
        "model_only",
        "Visible only to the model",
        Map::from_iter(
            json!({"type": "object", "properties": {}, "additionalProperties": false})
                .as_object()
                .unwrap()
                .clone(),
        ),
    );
    model_only.meta = Some(rmcp::model::MetaObject(Map::from_iter([(
        "ui".into(),
        json!({"visibility": ["model"]}),
    )])));
    let mut config = stdio_config();
    config.exposure_policy = McpExposurePolicy::Direct;
    let bindings = build_tool_catalog(
        &config,
        McpConnectionGeneration([9; 32]),
        vec![app_only, model_and_app, model_only],
    )
    .unwrap()
    .tools;
    let hidden = bindings
        .iter()
        .find(|binding| binding.remote_name == "app_only")
        .unwrap();
    assert!(!hidden.model_visible);
    assert_eq!(hidden.exposure, McpExposurePolicy::Hidden);
    let visible = bindings
        .iter()
        .find(|binding| binding.remote_name == "model_and_app")
        .unwrap();
    assert!(visible.model_visible);
    assert!(visible.app_visible);
    assert_eq!(visible.exposure, McpExposurePolicy::Direct);
    let model_only = bindings
        .iter()
        .find(|binding| binding.remote_name == "model_only")
        .unwrap();
    assert!(model_only.model_visible);
    assert!(!model_only.app_visible);
    assert_eq!(model_only.exposure, McpExposurePolicy::DirectModelOnly);
}

fn object_schema() -> Map<String, serde_json::Value> {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
        .as_object()
        .unwrap()
        .clone()
}

fn deep_schema(depth: usize) -> Map<String, serde_json::Value> {
    let mut schema = json!({"type": "string"});
    for _ in 0..depth {
        schema = json!({"type": "object", "properties": {"nested": schema}});
    }
    schema.as_object().unwrap().clone()
}

#[test]
fn one_bad_tool_is_skipped_and_the_rest_of_the_catalog_is_kept() {
    let oversized = Map::from_iter([(
        "description".to_string(),
        json!("x".repeat(hanafubuki_agent_tool_contracts::MAX_TOOL_SCHEMA_BYTES)),
    )]);
    let mut dropped_output = Tool::new("with_output", "Output schema too large", object_schema());
    dropped_output.output_schema = Some(Arc::new(oversized.clone()));
    let catalog = build_tool_catalog(
        &stdio_config(),
        McpConnectionGeneration([4; 32]),
        vec![
            Tool::new("first", "The first of two", object_schema()),
            Tool::new("oversized", "Schema too large", oversized),
            Tool::new("deep", "Schema too deep", deep_schema(40)),
            Tool::new("first", "A later duplicate", object_schema()),
            dropped_output,
            Tool::new("last", "Kept", object_schema()),
        ],
    )
    .unwrap();
    let kept = catalog
        .tools
        .iter()
        .map(|tool| {
            (
                tool.remote_name.as_str(),
                tool.specification.description.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kept,
        [
            ("first", "The first of two"),
            ("last", "Kept"),
            ("with_output", "Output schema too large"),
        ]
    );
    let with_output = &catalog.tools[2];
    assert_eq!(with_output.output_schema, None);
    let issues = catalog
        .issues
        .iter()
        .map(|issue| (issue.remote_name.as_str(), issue.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        issues,
        [
            ("oversized", McpCatalogIssueKind::InvalidInputSchema),
            ("deep", McpCatalogIssueKind::InvalidInputSchema),
            ("first", McpCatalogIssueKind::DuplicateName),
            ("with_output", McpCatalogIssueKind::OutputSchemaDropped),
        ]
    );
    assert!(catalog
        .tools
        .iter()
        .all(|tool| tool.catalog_generation == catalog.generation));
}

#[test]
fn a_listing_without_usable_tools_is_an_empty_catalog_not_an_error() {
    let catalog = build_tool_catalog(
        &stdio_config(),
        McpConnectionGeneration([5; 32]),
        vec![Tool::new("deep", "Schema too deep", deep_schema(40))],
    )
    .unwrap();
    assert!(catalog.tools.is_empty());
    assert_eq!(catalog.issues.len(), 1);
    assert!(catalog.issues[0].kind.skips_tool());
}

#[test]
fn an_over_long_description_is_truncated_on_a_char_boundary() {
    // One ASCII byte shifts every two-byte character so the limit falls
    // inside one.
    let description = format!("a{}", "é".repeat(MAX_MCP_TOOL_DESCRIPTION_BYTES));
    let catalog = build_tool_catalog(
        &stdio_config(),
        McpConnectionGeneration([6; 32]),
        vec![Tool::new("long", description.clone(), object_schema())],
    )
    .unwrap();
    let kept = &catalog.tools[0].specification.description;
    assert_eq!(kept.len(), MAX_MCP_TOOL_DESCRIPTION_BYTES - 1);
    assert!(description.starts_with(kept.as_str()));
    assert_eq!(
        catalog.issues,
        [McpCatalogIssue {
            remote_name: "long".into(),
            kind: McpCatalogIssueKind::DescriptionTruncated,
        }]
    );
    let long_name = "n".repeat(MAX_MCP_CATALOG_ISSUE_NAME_BYTES * 2);
    let catalog = build_tool_catalog(
        &stdio_config(),
        McpConnectionGeneration([6; 32]),
        vec![Tool::new(long_name, "Schema too deep", deep_schema(40))],
    )
    .unwrap();
    assert_eq!(
        catalog.issues[0].remote_name.len(),
        MAX_MCP_CATALOG_ISSUE_NAME_BYTES
    );
}

#[test]
fn dotted_server_ids_and_tool_names_are_sanitized_per_segment() {
    let mut config = stdio_config();
    config.id = McpServerId::parse("acme.tools").unwrap();
    let long = format!("{}.{}", "a".repeat(100), "b".repeat(100));
    let catalog = build_tool_catalog(
        &config,
        McpConnectionGeneration([8; 32]),
        vec![
            Tool::new("repo.search", "Dotted", object_schema()),
            Tool::new("plain", "Plain", object_schema()),
            Tool::new("repo_search", "Sanitizes like repo.search", object_schema()),
            Tool::new(long.clone(), "Over the identifier limit", object_schema()),
        ],
    )
    .unwrap();
    assert_eq!(
        catalog.issues,
        [McpCatalogIssue {
            remote_name: "repo_search".into(),
            kind: McpCatalogIssueKind::DuplicateName,
        }]
    );
    let names = catalog
        .tools
        .iter()
        .map(|tool| {
            assert_eq!(tool.specification.name.namespace(), ["mcp", "acme_tools"]);
            (
                tool.remote_name.as_str(),
                tool.specification.name.local_name().to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(names["repo.search"], "repo_search");
    assert_eq!(names["plain"], "plain");
    let fitted = &names[long.as_str()];
    assert_eq!(
        fitted.len(),
        hanafubuki_agent_tool_contracts::MAX_TOOL_IDENTIFIER_BYTES
    );
    assert!(fitted.starts_with(&"a".repeat(100)));
    assert!(fitted[115..].starts_with('_'));
}
