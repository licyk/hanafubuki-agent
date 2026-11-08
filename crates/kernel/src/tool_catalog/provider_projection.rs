use super::*;

pub(super) fn provider_supports_spec(
    specification: &ToolSpec,
    capabilities: hanafubuki_agent_tool_contracts::ToolProviderCapabilities,
) -> bool {
    match specification {
        ToolSpec::Function(_) => capabilities.function_tools,
        ToolSpec::Namespace(_) => capabilities.function_tools,
        ToolSpec::ToolSearch(_) => capabilities.native_tool_search,
        ToolSpec::HostedWebSearch(_) => capabilities.hosted_web_search,
        ToolSpec::Freeform(_) => capabilities.freeform_tools,
    }
}

pub(super) fn defer_oversized_direct_catalog(
    registrations: &mut [AgentPlannedTool],
) -> Result<(), AgentToolCatalogError> {
    let mut visible_count = 0usize;
    let mut visible_bytes = 0usize;
    for registration in registrations.iter() {
        if !matches!(
            registration.exposure,
            ToolExposure::Direct | ToolExposure::DirectModelOnly
        ) || !registration.is_first_party()
        {
            continue;
        }
        let bytes = serde_json::to_vec(&registration.provider_spec)
            .map_err(|_| AgentToolCatalogError::InvalidSchema)?
            .len();
        visible_count += 1;
        visible_bytes = visible_bytes.saturating_add(bytes);
    }
    if visible_count > MAX_AGENT_DIRECT_TOOL_ENTRIES
        || visible_bytes > MAX_AGENT_DIRECT_TOOL_SCHEMA_BYTES
    {
        return Err(AgentToolCatalogError::SchemaBytesExceeded {
            limit: MAX_AGENT_DIRECT_TOOL_SCHEMA_BYTES,
        });
    }
    let mut providers = std::collections::BTreeMap::<AgentToolProviderId, Vec<usize>>::new();
    for (index, registration) in registrations.iter().enumerate() {
        if registration.is_sourced()
            && matches!(
                registration.exposure,
                ToolExposure::Direct | ToolExposure::DirectModelOnly
            )
        {
            providers
                .entry(registration.provider_id.clone())
                .or_default()
                .push(index);
        }
    }
    let mut offsets = std::collections::BTreeMap::<AgentToolProviderId, usize>::new();
    loop {
        let mut advanced = false;
        for (provider, indexes) in &providers {
            let offset = offsets.entry(provider.clone()).or_default();
            let Some(index) = indexes.get(*offset).copied() else {
                continue;
            };
            *offset += 1;
            advanced = true;
            let bytes = serde_json::to_vec(&registrations[index].provider_spec)
                .map_err(|_| AgentToolCatalogError::InvalidSchema)?
                .len();
            if visible_count < MAX_AGENT_DIRECT_TOOL_ENTRIES
                && visible_bytes.saturating_add(bytes) <= MAX_AGENT_DIRECT_TOOL_SCHEMA_BYTES
            {
                visible_count += 1;
                visible_bytes = visible_bytes.saturating_add(bytes);
            } else {
                registrations[index].exposure = ToolExposure::Deferred;
            }
        }
        if !advanced {
            break;
        }
    }
    Ok(())
}

pub(super) fn plan_search_registration(
    exposure: ToolExposure,
    description: String,
) -> Result<AgentPlannedTool, AgentToolCatalogError> {
    let model_spec = ModelToolSpec {
        name: hanafubuki_agent_model_api::FALLBACK_TOOL_SEARCH_NAME.to_string(),
        description,
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "minLength": 1, "maxLength": 512},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_AGENT_TOOL_SEARCH_RESULTS}
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        strict: true,
    };
    AgentPlannedTool::plan_search(
        AgentToolProviderId::parse("agent-plan".to_string())
            .map_err(|_| AgentToolCatalogError::InvalidToolName)?,
        model_spec,
        AgentToolExecutionClass::ConcurrentRead,
        exposure,
    )
}

pub(super) fn tool_search_description(tools: &[AgentPlannedTool]) -> String {
    const MAXIMUM_SOURCE_BYTES: usize = 64 * 1024;
    let mut sources = std::collections::BTreeMap::<String, Option<String>>::new();
    for info in tools
        .iter()
        .filter(|tool| tool.exposure == ToolExposure::Deferred)
        .filter_map(|tool| tool.search_info.as_ref())
    {
        sources
            .entry(info.source_name.clone())
            .and_modify(|description| {
                if description.is_none() {
                    *description = info.source_description.clone();
                }
            })
            .or_insert_with(|| info.source_description.clone());
    }
    let mut listing = String::new();
    for (name, description) in sources {
        let line = description.map_or_else(
            || format!("- {name}\n"),
            |description| format!("- {name}: {description}\n"),
        );
        if listing.len().saturating_add(line.len()) > MAXIMUM_SOURCE_BYTES {
            break;
        }
        listing.push_str(&line);
    }
    if listing.is_empty() {
        listing.push_str("- None currently enabled.\n");
    }
    format!(
        "# Tool discovery\n\nSearch deferred tool metadata with BM25 and expose matching tools for the next model call. The default result limit is {DEFAULT_AGENT_TOOL_SEARCH_RESULTS}.\n\nAvailable sources:\n{listing}\nSome tools are not provided upfront. Use `tool_search` to discover required tools. For MCP tool discovery, use `tool_search` instead of `list_mcp_resources` or `list_mcp_resource_templates`."
    )
}

pub(super) fn set_tool_spec_deferred(specification: &mut ToolSpec, deferred: bool) {
    match specification {
        ToolSpec::Function(function) => function.deferred = deferred,
        ToolSpec::Namespace(namespace) => {
            for function in &mut namespace.tools {
                function.deferred = deferred;
            }
        }
        ToolSpec::ToolSearch(_) | ToolSpec::HostedWebSearch(_) | ToolSpec::Freeform(_) => {}
    }
}

pub(super) fn tool_spec_sort_key(specification: &ToolSpec) -> String {
    specification
        .name()
        .map(ToolName::flattened_alias)
        .unwrap_or_else(|| specification.description().to_string())
}

/// Lowers a spec's schemas to the portable keywords, for a protocol that
/// does not take a JSON Schema whole (`strict_json_schema`).
pub(super) fn lower_to_portable_schemas(spec: &mut ToolSpec) {
    match spec {
        ToolSpec::Function(function) => {
            function.input_schema = portable_model_schema(&function.input_schema);
        }
        ToolSpec::Namespace(namespace) => {
            for function in &mut namespace.tools {
                function.input_schema = portable_model_schema(&function.input_schema);
            }
        }
        ToolSpec::ToolSearch(search) => {
            search.parameters = portable_model_schema(&search.parameters);
        }
        ToolSpec::HostedWebSearch(_) | ToolSpec::Freeform(_) => {}
    }
}

pub(super) fn portable_model_schema(schema: &Value) -> Value {
    let Value::Object(object) = schema else {
        return schema.clone();
    };
    let mut portable = serde_json::Map::new();
    for (key, value) in object {
        match key.as_str() {
            "$ref" | "type" | "description" | "encrypted" | "enum" | "required" => {
                portable.insert(key.clone(), value.clone());
            }
            "const" => {
                portable.insert("enum".to_string(), Value::Array(vec![value.clone()]));
            }
            "items" | "additionalProperties" => {
                portable.insert(key.clone(), portable_model_schema(value));
            }
            "properties" | "$defs" | "definitions" => {
                if let Some(entries) = value.as_object() {
                    portable.insert(
                        key.clone(),
                        Value::Object(
                            entries
                                .iter()
                                .map(|(name, schema)| (name.clone(), portable_model_schema(schema)))
                                .collect(),
                        ),
                    );
                }
            }
            "anyOf" | "oneOf" | "allOf" => {
                if let Some(variants) = value.as_array() {
                    portable.insert(
                        key.clone(),
                        Value::Array(variants.iter().map(portable_model_schema).collect()),
                    );
                }
            }
            _ => {}
        }
    }
    Value::Object(portable)
}
