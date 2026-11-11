use super::*;

pub(crate) type ToolLifecycleHookEntry = Arc<
    hanafubuki_agent_contracts::RegistryEntry<
        hanafubuki_agent_contracts::ToolLifecycleHookRegistration,
    >,
>;

#[derive(Clone)]
pub struct AgentToolPlan {
    pub revision: AgentToolCatalogRevision,
    registrations: Vec<AgentPlannedTool>,
    additional_model_specs: Vec<ToolSpec>,
    parallel_tool_calls: bool,
    native_tool_search: bool,
    leases: Vec<Arc<dyn ToolPlanLease>>,
    approval_rules: Option<hanafubuki_agent_contracts::ApprovalRuleSnapshot>,
    tool_lifecycle_hooks: Vec<ToolLifecycleHookEntry>,
    omissions: Vec<AgentToolOmission>,
    search_index: Arc<AgentToolSearchIndex>,
    admitted_deferred_tools: BTreeSet<AgentToolSelection>,
    wire_names: ProviderToolTable,
}

impl std::fmt::Debug for AgentToolPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentToolPlan")
            .field("revision", &self.revision)
            .field("registrations", &self.registrations)
            .field("additional_model_specs", &self.additional_model_specs)
            .field("parallel_tool_calls", &self.parallel_tool_calls)
            .field("native_tool_search", &self.native_tool_search)
            .field("leases", &self.leases.len())
            .field("omissions", &self.omissions)
            .field("searchable_tools", &self.search_index.document_count())
            .field("admitted_deferred_tools", &self.admitted_deferred_tools)
            .finish()
    }
}

impl PartialEq for AgentToolPlan {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.registrations == other.registrations
            && self.additional_model_specs == other.additional_model_specs
            && self.parallel_tool_calls == other.parallel_tool_calls
            && self.native_tool_search == other.native_tool_search
            && self.omissions == other.omissions
            && self.admitted_deferred_tools == other.admitted_deferred_tools
    }
}

impl Eq for AgentToolPlan {}

impl AgentToolPlan {
    /// A plan of `registrations` with no additional specs. Plans come from the
    /// planner ([`AgentToolPlanner::build`]), which validates each tool's
    /// specs and gives each its wire name first; this builds one from tools
    /// the caller vouches for, named as for a protocol without namespaces.
    #[cfg(test)]
    pub(crate) fn build(
        mut registrations: Vec<AgentPlannedTool>,
    ) -> Result<Self, AgentToolCatalogError> {
        let capabilities = hanafubuki_agent_tool_contracts::ToolProviderCapabilities {
            maximum_tool_name_bytes: hanafubuki_agent_tool_contracts::MAX_TOOL_IDENTIFIER_BYTES,
            ..hanafubuki_agent_model_api::ModelApiProtocol::ChatCompletions.tool_capabilities()
        };
        assign_provider_names(&mut registrations, capabilities, &mut Vec::new())?;
        Self::build_with_additional_specs(
            registrations,
            Vec::new(),
            false,
            Vec::new(),
            BTreeSet::new(),
        )
    }

    /// Freezes the planner's tools. Their schemas and provider specs were
    /// validated by the planner (`validate_planned_tool`) and their wire
    /// names made unique by `assign_provider_names`, so neither is checked
    /// again; what only the whole plan can check is: the entry count, unique
    /// identities, the total schema bytes and the additional specs.
    pub(crate) fn build_with_additional_specs(
        mut registrations: Vec<AgentPlannedTool>,
        mut additional_model_specs: Vec<ToolSpec>,
        native_tool_search: bool,
        mut omissions: Vec<AgentToolOmission>,
        admitted_deferred_tools: BTreeSet<AgentToolSelection>,
    ) -> Result<Self, AgentToolCatalogError> {
        if registrations.len() > MAX_AGENT_TOOL_CATALOG_ENTRIES {
            return Err(AgentToolCatalogError::TooManyTools {
                limit: MAX_AGENT_TOOL_CATALOG_ENTRIES,
            });
        }
        registrations.sort_by(|left, right| {
            left.model_spec
                .name
                .cmp(&right.model_spec.name)
                .then_with(|| left.provider_id.cmp(&right.provider_id))
                .then_with(|| left.tool_id.cmp(&right.tool_id))
        });
        let mut ids = HashSet::with_capacity(registrations.len());
        let mut schema_bytes = 0usize;
        for registration in &registrations {
            if registration.is_first_party()
                && registration.model_spec.name != registration.tool_id.as_str()
            {
                return Err(AgentToolCatalogError::InvalidToolName);
            }
            if !ids.insert((
                registration.provider_id.clone(),
                registration.tool_id.clone(),
            )) {
                return Err(AgentToolCatalogError::DuplicateToolId);
            }
            schema_bytes = schema_bytes.saturating_add(
                serde_json::to_vec(&registration.model_spec)
                    .map_err(|_| AgentToolCatalogError::InvalidSchema)?
                    .len(),
            );
        }
        if schema_bytes > MAX_AGENT_TOOL_CATALOG_SCHEMA_BYTES {
            return Err(AgentToolCatalogError::SchemaBytesExceeded {
                limit: MAX_AGENT_TOOL_CATALOG_SCHEMA_BYTES,
            });
        }
        additional_model_specs.sort_by_key(tool_spec_sort_key);
        for specification in &additional_model_specs {
            specification
                .validate()
                .map_err(|_| AgentToolCatalogError::InvalidSchema)?;
        }
        let canonical = serde_json::to_vec(&(
            &registrations,
            &additional_model_specs,
            &admitted_deferred_tools,
        ))
        .map_err(|_| AgentToolCatalogError::InvalidSchema)?;
        let revision = AgentToolCatalogRevision(Sha256::digest(canonical).into());
        let search_index = AgentToolSearchIndexCache::default().get_or_build(&registrations)?;
        let wire_names = ProviderToolTable::index(&registrations);
        Ok(Self {
            revision,
            registrations,
            additional_model_specs,
            parallel_tool_calls: true,
            native_tool_search,
            leases: Vec::new(),
            approval_rules: None,
            tool_lifecycle_hooks: Vec::new(),
            omissions: {
                omissions.truncate(MAX_AGENT_TOOL_OMISSIONS);
                omissions
            },
            search_index,
            admitted_deferred_tools,
            wire_names,
        })
    }

    pub(super) fn with_leases(mut self, leases: Vec<Arc<dyn ToolPlanLease>>) -> Self {
        self.leases = leases;
        self
    }

    /// Freezes the approval rules this step's calls are judged by.
    pub(crate) fn with_approval_rules(
        mut self,
        rules: hanafubuki_agent_contracts::ApprovalRuleSnapshot,
    ) -> Self {
        self.approval_rules = Some(rules);
        self
    }

    /// Freezes the tool lifecycle hooks this step's calls are run with, in
    /// the order they are asked.
    pub(crate) fn with_tool_lifecycle_hooks(mut self, hooks: Vec<ToolLifecycleHookEntry>) -> Self {
        self.tool_lifecycle_hooks = hooks;
        self
    }

    /// The tool lifecycle hooks frozen with the plan, in order.
    pub(crate) fn tool_lifecycle_hooks(&self) -> &[ToolLifecycleHookEntry] {
        &self.tool_lifecycle_hooks
    }

    /// The approval rules frozen with the plan; `None` only for plans built
    /// outside the kernel's planning step.
    pub fn approval_rules(&self) -> Option<&hanafubuki_agent_contracts::ApprovalRuleSnapshot> {
        self.approval_rules.as_ref()
    }

    /// The leases the plan's sources returned, kept alive with the plan.
    pub fn leases(&self) -> &[Arc<dyn ToolPlanLease>] {
        &self.leases
    }

    pub fn omissions(&self) -> &[AgentToolOmission] {
        &self.omissions
    }

    pub(super) fn with_search_index(
        mut self,
        cache: &AgentToolSearchIndexCache,
    ) -> Result<Self, AgentToolCatalogError> {
        self.search_index = cache.get_or_build(&self.registrations)?;
        Ok(self)
    }

    #[cfg(test)]
    pub(super) fn shares_search_index_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.search_index, &other.search_index)
    }

    pub fn with_parallel_tool_calls(mut self, enabled: bool) -> Self {
        self.parallel_tool_calls = enabled;
        self
    }

    pub fn parallel_tool_calls(&self) -> bool {
        self.parallel_tool_calls
    }

    pub fn native_tool_search(&self) -> bool {
        self.native_tool_search
    }

    pub fn model_specs(&self) -> Vec<ToolSpec> {
        let mut specifications = self
            .registrations
            .iter()
            .filter(|registration| {
                matches!(
                    registration.exposure,
                    ToolExposure::Direct | ToolExposure::DirectModelOnly
                )
            })
            .map(|registration| registration.provider_spec.clone())
            .collect::<Vec<_>>();
        specifications.extend(self.additional_model_specs.clone());
        specifications
    }

    /// The tool a wire name resolves to, by lookup in the request's names:
    /// a Direct tool, or a Deferred one the provider's own search admitted.
    pub fn resolve(
        &self,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<&AgentPlannedTool, AgentToolCatalogError> {
        self.named(namespace, name)
            .filter(|registration| {
                matches!(
                    registration.exposure,
                    ToolExposure::Direct | ToolExposure::DirectModelOnly
                ) || (self.native_tool_search
                    && registration.exposure == ToolExposure::Deferred
                    && self
                        .admitted_deferred_tools
                        .contains(&AgentToolSelection::from_planned_tool(registration)))
            })
            .ok_or_else(|| AgentToolCatalogError::ToolNotAdvertised(name.to_string()))
    }

    fn named(&self, namespace: Option<&str>, name: &str) -> Option<&AgentPlannedTool> {
        self.wire_names
            .get(namespace, name)
            .map(|index| &self.registrations[index])
    }

    pub fn resolve_call(
        &self,
        call: &hanafubuki_agent_model_api::ModelToolCall,
    ) -> Result<&AgentPlannedTool, AgentToolCatalogError> {
        match call.origin() {
            hanafubuki_agent_model_api::ModelToolCallOrigin::Function
            | hanafubuki_agent_model_api::ModelToolCallOrigin::Freeform => {
                self.resolve(call.namespace.as_deref(), &call.name)
            }
            hanafubuki_agent_model_api::ModelToolCallOrigin::ToolSearch => {
                if !self.native_tool_search
                    || call.name != hanafubuki_agent_model_api::NATIVE_TOOL_SEARCH_NAME
                {
                    return Err(AgentToolCatalogError::ToolNotAdvertised(call.name.clone()));
                }
                self.registrations
                    .iter()
                    .find(|registration| {
                        matches!(registration.binding, AgentPlannedToolBinding::PlanSearch)
                    })
                    .ok_or_else(|| AgentToolCatalogError::ToolNotAdvertised(call.name.clone()))
            }
        }
    }

    /// Returns a known Deferred registration that the current sampling step
    /// did not advertise or admit.
    ///
    /// This is not an execution capability. It exists only so Agent core can
    /// tell the model to discover the tool first. Unknown, mode-hidden,
    /// provider-incompatible, and Hidden registrations are not returned: they
    /// stay indistinguishable from absent tools, and every such call gets the
    /// one identical "no tool named ... is available in this step" error and
    /// never runs.
    pub(crate) fn unselected_deferred_call(
        &self,
        call: &hanafubuki_agent_model_api::ModelToolCall,
    ) -> Option<&AgentPlannedTool> {
        if !matches!(
            call.origin(),
            hanafubuki_agent_model_api::ModelToolCallOrigin::Function
                | hanafubuki_agent_model_api::ModelToolCallOrigin::Freeform
        ) {
            return None;
        }
        self.named(call.namespace.as_deref(), &call.name)
            .filter(|registration| registration.exposure == ToolExposure::Deferred)
    }

    pub fn registrations(&self) -> &[AgentPlannedTool] {
        &self.registrations
    }

    /// The names of the plan's tools (the ones scope patterns match): every
    /// tool it offers directly or through a search, not the Hidden ones.
    pub(crate) fn offered_tool_names(&self) -> BTreeSet<String> {
        self.registrations
            .iter()
            .filter(|registration| registration.exposure != ToolExposure::Hidden)
            .map(|registration| registration.model_spec.name.clone())
            .collect()
    }

    pub fn deferred_registrations(&self) -> impl Iterator<Item = &AgentPlannedTool> {
        self.registrations
            .iter()
            .filter(|registration| registration.exposure == ToolExposure::Deferred)
    }

    pub fn search_deferred(
        &self,
        query: &str,
        requested_limit: usize,
    ) -> Vec<AgentToolSearchMatch> {
        self.search_index.search(query, requested_limit)
    }

    pub fn selected_tools_pollute_memory(&self, selections: &[AgentToolSelection]) -> bool {
        selections.iter().any(|selection| {
            self.registrations.iter().any(|registration| {
                AgentToolSelection::from_planned_tool(registration) == *selection
                    && registration.memory_context_effect
                        == AgentMemoryContextEffect::PollutesContribution
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentToolSearchMatch {
    pub selection: AgentToolSelection,
    pub specification: ToolSpec,
    pub model_name: String,
    pub description: String,
    pub score_milli: u32,
}
