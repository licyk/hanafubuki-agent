use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentToolCatalogContext {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub session_revision: crate::AgentRevision,
    pub sampling_step: u32,
    pub mode: crate::AgentSessionMode,
    pub input_capabilities: hanafubuki_agent_model_api::ModelInputCapabilities,
    pub provider_capabilities: hanafubuki_agent_tool_contracts::ToolProviderCapabilities,
    pub selected_deferred_tools: BTreeSet<AgentToolSelection>,
    pub memory_contribution_enabled: bool,
    /// The compression tools the step plans: none unless the session's
    /// strategy is `hierarchical`, then the five core ones and those of the
    /// opt-in features that are on.
    pub compression_tools: Vec<crate::AgentCoreTool>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolSelection {
    pub provider_id: AgentToolProviderId,
    pub tool_id: AgentToolId,
    pub generation: AgentToolGeneration,
}

/// One contributor's planned tools for a step.
#[derive(Clone, Default)]
pub(crate) struct AgentToolContribution {
    pub tools: Vec<AgentPlannedTool>,
    pub omissions: Vec<AgentToolOmission>,
    /// Kept by the plan and handed to every call planned from it, so the
    /// contributor's tools run against the state they were planned from.
    pub lease: Option<Arc<dyn ToolPlanLease>>,
}

impl std::fmt::Debug for AgentToolContribution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentToolContribution")
            .field("tools", &self.tools)
            .field("omissions", &self.omissions)
            .field("has_lease", &self.lease.is_some())
            .finish()
    }
}

impl AgentToolContribution {
    pub fn from_tools(tools: Vec<AgentPlannedTool>) -> Self {
        Self {
            tools,
            omissions: Vec::new(),
            lease: None,
        }
    }
}

/// What the planner asks for a step's tools: the kernel's own tools, the
/// registered tools and each tool source.
pub(crate) trait AgentToolContributor: Send + Sync {
    fn id(&self) -> &str;

    fn trust(&self) -> AgentToolTrust {
        AgentToolTrust::Trusted
    }

    fn contribute(
        &self,
        context: AgentToolCatalogContext,
        cancellation: CancellationToken,
    ) -> AgentToolContributionFuture<'_>;
}

#[derive(Default)]
pub(crate) struct AgentToolPlanner {
    contributors: Vec<Arc<dyn AgentToolContributor>>,
    search_cache: AgentToolSearchIndexCache,
    spec_cache: AgentToolSpecCache,
}

impl AgentToolPlanner {
    pub fn new(contributors: Vec<Arc<dyn AgentToolContributor>>) -> Self {
        Self {
            contributors,
            search_cache: AgentToolSearchIndexCache::default(),
            spec_cache: AgentToolSpecCache::default(),
        }
    }

    pub fn with_search_cache(mut self, search_cache: AgentToolSearchIndexCache) -> Self {
        self.search_cache = search_cache;
        self
    }

    pub fn with_spec_cache(mut self, spec_cache: AgentToolSpecCache) -> Self {
        self.spec_cache = spec_cache;
        self
    }

    /// Adds the tool sources plugins registered, from one snapshot of the
    /// `ToolSourceRegistry` taken for the sampling step. Each source keeps
    /// its own trust: an optional one that fails is omitted.
    pub fn with_sources(mut self, sources: RegistrySnapshot<Arc<dyn AgentToolSource>>) -> Self {
        self.contributors.extend(
            sources
                .entries()
                .iter()
                .filter(|entry| !entry.is_retired())
                .map(|entry| {
                    Arc::new(SourcedAgentToolContributor::new(entry.clone()))
                        as Arc<dyn AgentToolContributor>
                }),
        );
        self
    }

    pub async fn build(
        &self,
        context: AgentToolCatalogContext,
        cancellation: CancellationToken,
    ) -> Result<AgentToolPlan, AgentToolCatalogError> {
        let mut registrations = Vec::new();
        let mut omissions = Vec::new();
        let mut leases = Vec::new();
        for contributor in &self.contributors {
            if cancellation.is_cancelled() {
                return Err(AgentToolCatalogError::Cancelled);
            }
            let contribution = contributor
                .contribute(context.clone(), cancellation.clone())
                .await;
            let mut contribution = match contribution {
                Ok(contribution) => contribution,
                Err(AgentToolCatalogError::Cancelled) => {
                    return Err(AgentToolCatalogError::Cancelled);
                }
                Err(error) if contributor.trust() == AgentToolTrust::OptionalExternal => {
                    push_optional_omission(
                        &mut omissions,
                        contributor.id(),
                        None,
                        format!("contributor failed: {error}"),
                    );
                    continue;
                }
                Err(error) => {
                    return Err(AgentToolCatalogError::ProviderFailed(format!(
                        "{}: {error}",
                        contributor.id()
                    )));
                }
            };
            registrations.append(&mut contribution.tools);
            omissions.append(&mut contribution.omissions);
            leases.extend(contribution.lease);
        }
        registrations.retain(|registration| {
            if let Some(handler) = &registration.handler {
                if context.mode == crate::AgentSessionMode::Plan
                    && handler.handler().descriptor().effect
                        != hanafubuki_agent_contracts::ToolEffect::ReadOnly
                {
                    return false;
                }
            } else if let AgentPlannedToolBinding::Core(tool) = registration.binding {
                if !tool.allowed_in_mode(context.mode) {
                    return false;
                }
            }
            provider_supports_spec(&registration.provider_spec, context.provider_capabilities)
        });
        registrations.sort_by(|left, right| {
            left.model_spec
                .name
                .cmp(&right.model_spec.name)
                .then_with(|| left.provider_id.cmp(&right.provider_id))
                .then_with(|| left.tool_id.cmp(&right.tool_id))
        });
        for registration in &mut registrations {
            let selected = AgentToolSelection::from_planned_tool(registration);
            if registration.exposure == ToolExposure::Deferred
                && context.selected_deferred_tools.contains(&selected)
                && !context.provider_capabilities.native_tool_search
            {
                registration.exposure = ToolExposure::Direct;
            }
        }
        let mut planned = Vec::with_capacity(registrations.len());
        for registration in registrations {
            if let Err(error) = validate_planned_tool(&registration, &self.spec_cache) {
                if registration.trust != AgentToolTrust::OptionalExternal {
                    return Err(error);
                }
                push_optional_omission(
                    &mut omissions,
                    registration.provider_id.as_str(),
                    Some(registration.model_spec.name.clone()),
                    format!("invalid tool: {error}"),
                );
                continue;
            }
            planned.push(registration);
        }
        registrations = planned;
        assign_provider_names(
            &mut registrations,
            context.provider_capabilities,
            &mut omissions,
        )?;
        for registration in &mut registrations {
            registration.refresh_search_info();
        }
        defer_oversized_direct_catalog(&mut registrations)?;
        let mut additional_model_specs = Vec::new();
        let has_native_deferred_tools = context.provider_capabilities.native_tool_search
            && registrations
                .iter()
                .any(|registration| registration.exposure == ToolExposure::Deferred);
        if has_native_deferred_tools {
            let description = tool_search_description(&registrations);
            additional_model_specs.push(ToolSpec::ToolSearch(ToolSearchSpec {
                execution: "client".to_string(),
                description: description.clone(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "minLength": 1, "maxLength": 512},
                        "limit": {"type": "integer", "minimum": 1, "maximum": MAX_AGENT_TOOL_SEARCH_RESULTS}
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            }));
            registrations.push(plan_search_registration(ToolExposure::Hidden, description)?);
        } else if registrations
            .iter()
            .any(|registration| registration.exposure == ToolExposure::Deferred)
        {
            let description = tool_search_description(&registrations);
            registrations.push(plan_search_registration(ToolExposure::Direct, description)?);
        }
        if !context.provider_capabilities.strict_json_schema {
            let specifications = registrations
                .iter_mut()
                .map(|registration| &mut registration.provider_spec)
                .chain(&mut additional_model_specs);
            for specification in specifications {
                lower_to_portable_schemas(specification);
            }
        }
        AgentToolPlan::build_with_additional_specs(
            registrations,
            additional_model_specs,
            has_native_deferred_tools,
            omissions,
            context.selected_deferred_tools,
        )
        .and_then(|plan| plan.with_search_index(&self.search_cache))
        .map(|plan| {
            plan.with_parallel_tool_calls(context.provider_capabilities.parallel_tool_calls)
                .with_leases(leases)
        })
    }
}

/// The one place a planned tool's schema and provider spec are validated:
/// once per tool identity (see [`AgentToolSpecCache`]), for every tool the
/// step plans, kernel, registered and sourced alike. A sourced schema was
/// already bounded by its source (the MCP catalog validates each tool's); the
/// kernel holds every source to the same bound here. The calls planned from
/// it validate their arguments against the schema without validating it
/// again.
fn validate_planned_tool(
    tool: &AgentPlannedTool,
    spec_cache: &AgentToolSpecCache,
) -> Result<(), AgentToolCatalogError> {
    spec_cache.validate_once(tool, || {
        validate_json_schema(&tool.model_spec.input_schema)
            .map_err(|_| AgentToolCatalogError::InvalidSchema)?;
        if let Some(description) = &tool.namespace_description {
            hanafubuki_agent_tool_contracts::validate_description(description)
                .map_err(|_| AgentToolCatalogError::InvalidSchema)?;
        }
        tool.provider_spec
            .validate()
            .map_err(|_| AgentToolCatalogError::InvalidSchema)
    })?;
    if matches!(tool.binding, AgentPlannedToolBinding::Handler) && tool.handler.is_none() {
        return Err(AgentToolCatalogError::InvalidRuntimeBinding);
    }
    Ok(())
}

pub(super) fn push_optional_omission(
    omissions: &mut Vec<AgentToolOmission>,
    contributor: &str,
    tool_name: Option<String>,
    reason: String,
) {
    let bounded_reason = reason.chars().take(512).collect::<String>();
    log::warn!(
        "Agent tool plan note: contributor={contributor} tool={} reason={bounded_reason}",
        tool_name.as_deref().unwrap_or("<catalog>")
    );
    if omissions.len() < MAX_AGENT_TOOL_OMISSIONS {
        omissions.push(AgentToolOmission {
            contributor: contributor.chars().take(128).collect(),
            tool_name: tool_name.map(|name| name.chars().take(256).collect()),
            reason: bounded_reason,
        });
    }
}
