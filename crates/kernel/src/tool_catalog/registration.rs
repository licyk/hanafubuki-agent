use super::*;

pub use hanafubuki_agent_contracts::AgentMemoryContextEffect;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlannedTool {
    pub provider_id: AgentToolProviderId,
    pub tool_id: AgentToolId,
    pub generation: AgentToolGeneration,
    pub model_spec: ModelToolSpec,
    pub provider_spec: ToolSpec,
    /// Bounded provider-owned description used only when the provider can
    /// expose a native namespace. It is part of the frozen catalog revision.
    pub namespace_description: Option<String>,
    pub exposure: ToolExposure,
    pub binding: AgentPlannedToolBinding,
    pub execution_class: AgentToolExecutionClass,
    pub presentation: AgentToolPresentation,
    pub memory_context_effect: AgentMemoryContextEffect,
    pub trust: AgentToolTrust,
    pub search_info: Option<AgentToolSearchInfo>,
    #[serde(skip)]
    pub handler: Option<AgentToolHandlerBinding>,
}

/// The handler a [`AgentPlannedToolBinding::Handler`] tool runs and the
/// registry entry its calls lease: the tool's own entry for a registered
/// tool, its source's for a sourced one. The frozen plan keeps the entry, so
/// a call can still lease it while its plugin stops; the next plan no longer
/// contains it.
#[derive(Clone)]
pub struct AgentToolHandlerBinding {
    handler: Arc<dyn AgentToolHandler>,
    owner: HandlerOwner,
}

#[derive(Clone)]
enum HandlerOwner {
    Registered(Arc<RegistryEntry<Arc<dyn AgentToolHandler>>>),
    Sourced(Arc<RegistryEntry<Arc<dyn AgentToolSource>>>),
}

impl AgentToolHandlerBinding {
    pub fn handler(&self) -> &Arc<dyn AgentToolHandler> {
        &self.handler
    }

    /// Planned by a tool source for one step rather than registered.
    pub fn is_sourced(&self) -> bool {
        matches!(self.owner, HandlerOwner::Sourced(_))
    }

    /// Pins the owning entry for one call; `None` once its plugin is
    /// stopping.
    pub fn acquire(&self, call: &CancellationToken) -> Option<EntryLease> {
        match &self.owner {
            HandlerOwner::Registered(entry) => entry.acquire(call),
            HandlerOwner::Sourced(entry) => entry.acquire(call),
        }
    }

    /// The plugin that registered the tool, or its source's for a sourced
    /// one.
    pub(crate) fn owner(&self) -> &str {
        match &self.owner {
            HandlerOwner::Registered(entry) => entry.owner(),
            HandlerOwner::Sourced(entry) => entry.owner(),
        }
    }

    /// Calls the handler at `point`; a panic is reported as a fault of the
    /// owning entry's plugin (the source's, for a sourced tool) and becomes
    /// [`hanafubuki_agent_contracts::PluginPanicked`].
    pub async fn guard_async<'a, F, Fut>(
        &'a self,
        point: hanafubuki_agent_contracts::ExtensionPoint,
        call: F,
    ) -> Result<Fut::Output, hanafubuki_agent_contracts::PluginPanicked>
    where
        F: FnOnce(&'a Arc<dyn AgentToolHandler>) -> Fut,
        Fut: std::future::Future,
    {
        let faults = match &self.owner {
            HandlerOwner::Registered(entry) => entry.faults(),
            HandlerOwner::Sourced(entry) => entry.faults(),
        };
        let handler = &self.handler;
        faults
            .guard_async(self.owner(), point, move || call(handler))
            .await
    }
}

impl std::fmt::Debug for AgentToolHandlerBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentToolHandlerBinding")
            .field("owner", &self.owner())
            .field("sourced", &self.is_sourced())
            .finish_non_exhaustive()
    }
}

impl PartialEq for AgentToolHandlerBinding {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.handler, &other.handler)
            && match (&self.owner, &other.owner) {
                (HandlerOwner::Registered(left), HandlerOwner::Registered(right)) => {
                    Arc::ptr_eq(left, right)
                }
                (HandlerOwner::Sourced(left), HandlerOwner::Sourced(right)) => {
                    Arc::ptr_eq(left, right)
                }
                _ => false,
            }
    }
}

impl Eq for AgentToolHandlerBinding {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolTrust {
    Trusted,
    RequiredExternal,
    OptionalExternal,
}

impl From<ToolSourceTrust> for AgentToolTrust {
    fn from(trust: ToolSourceTrust) -> Self {
        match trust {
            ToolSourceTrust::Required => Self::RequiredExternal,
            ToolSourceTrust::Optional => Self::OptionalExternal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPlannedToolBinding {
    /// One of the kernel's own tools, which the kernel runs itself.
    Core(crate::AgentCoreTool),
    /// A plugin's handler: registered in the tool registry, or planned by a
    /// tool source for one step.
    Handler,
    PlanSearch,
}

impl AgentToolSelection {
    pub fn from_planned_tool(tool: &AgentPlannedTool) -> Self {
        Self {
            provider_id: tool.provider_id.clone(),
            tool_id: tool.tool_id.clone(),
            generation: tool.generation,
        }
    }
}

impl AgentPlannedTool {
    /// A tool the application ships (a kernel or a registered tool), as
    /// opposed to one a tool source planned for the step.
    pub fn is_first_party(&self) -> bool {
        match self.binding {
            AgentPlannedToolBinding::Core(_) => true,
            AgentPlannedToolBinding::Handler => self
                .handler
                .as_ref()
                .is_some_and(|binding| !binding.is_sourced()),
            AgentPlannedToolBinding::PlanSearch => false,
        }
    }

    /// Planned by a tool source for this step.
    pub fn is_sourced(&self) -> bool {
        self.handler
            .as_ref()
            .is_some_and(AgentToolHandlerBinding::is_sourced)
    }

    /// A kernel tool. Its model-visible name must be the core tool's.
    pub(crate) fn fixed(
        provider_id: AgentToolProviderId,
        mut model_spec: ModelToolSpec,
        execution_class: AgentToolExecutionClass,
    ) -> Result<Self, AgentToolCatalogError> {
        let exposure = ToolExposure::Direct;
        model_spec.strict = false;
        let tool_id = AgentToolId::parse(model_spec.name.clone())
            .map_err(|_| AgentToolCatalogError::InvalidToolName)?;
        let core_tool = crate::AgentCoreTool::ALL
            .into_iter()
            .find(|tool| tool.name() == model_spec.name)
            .ok_or(AgentToolCatalogError::InvalidToolName)?;
        let presentation = AgentToolPresentation {
            action: core_tool.activity_action(),
            subject: None,
        };
        let memory_context_effect = AgentMemoryContextEffect::None;
        let provider_spec = ToolSpec::Function(FunctionToolSpec {
            name: ToolName::parse(model_spec.name.clone())
                .map_err(|_| AgentToolCatalogError::InvalidToolName)?,
            description: model_spec.description.clone(),
            input_schema: model_spec.input_schema.clone(),
            strict: false,
            deferred: false,
        });
        let canonical = serde_json::to_vec(&(
            provider_id.as_str(),
            tool_id.as_str(),
            &model_spec,
            &provider_spec,
            exposure,
            AgentPlannedToolBinding::Core(core_tool),
            execution_class,
            &presentation,
            memory_context_effect,
            AgentToolTrust::Trusted,
        ))
        .map_err(|_| AgentToolCatalogError::InvalidSchema)?;
        let generation = AgentToolGeneration(Sha256::digest(canonical).into());
        let mut tool = Self {
            provider_id,
            tool_id,
            generation,
            model_spec,
            provider_spec,
            namespace_description: None,
            exposure,
            binding: AgentPlannedToolBinding::Core(core_tool),
            execution_class,
            presentation,
            memory_context_effect,
            trust: AgentToolTrust::Trusted,
            search_info: None,
            handler: None,
        };
        tool.refresh_search_info();
        Ok(tool)
    }

    /// A tool registered by a plugin. Its model-visible spec is built exactly
    /// as a kernel tool's, so where a tool lives does not change what the
    /// model sees.
    pub fn registered(
        provider_id: AgentToolProviderId,
        entry: Arc<RegistryEntry<Arc<dyn AgentToolHandler>>>,
        availability: hanafubuki_agent_contracts::ToolAvailability,
    ) -> Result<Self, AgentToolCatalogError> {
        let handler = entry.item().clone();
        let descriptor = handler.descriptor();
        let name = descriptor.name.flattened_alias();
        let tool_id =
            AgentToolId::parse(name.clone()).map_err(|_| AgentToolCatalogError::InvalidToolName)?;
        let model_spec = ModelToolSpec {
            name,
            description: descriptor.description.clone(),
            input_schema: descriptor.input_schema.clone(),
            strict: false,
        };
        let mut provider_spec = ToolSpec::Function(FunctionToolSpec {
            name: descriptor.name.clone(),
            description: descriptor.description.clone(),
            input_schema: descriptor.input_schema.clone(),
            strict: false,
            deferred: false,
        });
        if let hanafubuki_agent_contracts::ToolAvailability::Freeform(freeform) = availability {
            provider_spec = ToolSpec::Freeform(freeform);
        }
        let presentation = AgentToolPresentation {
            action: descriptor.action.clone(),
            subject: None,
        };
        let canonical = serde_json::to_vec(&(
            provider_id.as_str(),
            tool_id.as_str(),
            &model_spec,
            &provider_spec,
            descriptor.exposure,
            AgentPlannedToolBinding::Handler,
            descriptor.execution_class,
            &presentation,
            descriptor.memory_effect,
            AgentToolTrust::Trusted,
        ))
        .map_err(|_| AgentToolCatalogError::InvalidSchema)?;
        let mut tool = Self {
            provider_id,
            tool_id,
            generation: AgentToolGeneration(Sha256::digest(canonical).into()),
            model_spec,
            provider_spec,
            namespace_description: None,
            exposure: descriptor.exposure,
            binding: AgentPlannedToolBinding::Handler,
            execution_class: descriptor.execution_class,
            presentation,
            memory_context_effect: descriptor.memory_effect,
            trust: AgentToolTrust::Trusted,
            search_info: None,
            handler: Some(AgentToolHandlerBinding {
                handler: handler.clone(),
                owner: HandlerOwner::Registered(entry.clone()),
            }),
        };
        tool.refresh_search_info();
        Ok(tool)
    }

    /// A tool a source planned for this step, with the identity the source
    /// gave it. Its name is the flattened alias here; the request's wire
    /// name comes from `assign_provider_names`, a native namespace where the
    /// provider takes one.
    pub fn sourced(
        source: Arc<RegistryEntry<Arc<dyn AgentToolSource>>>,
        tool: SourcedTool,
        availability: hanafubuki_agent_contracts::ToolAvailability,
    ) -> Result<Self, AgentToolCatalogError> {
        let descriptor = tool.handler.descriptor();
        if !AgentActivityAction::is_valid(descriptor.action.as_str()) {
            return Err(AgentToolCatalogError::InvalidToolName);
        }
        let model_spec = ModelToolSpec {
            name: descriptor.name.flattened_alias(),
            description: descriptor.description.clone(),
            input_schema: descriptor.input_schema.clone(),
            strict: false,
        };
        let mut provider_spec = ToolSpec::Function(FunctionToolSpec {
            name: descriptor.name.clone(),
            description: descriptor.description.clone(),
            input_schema: descriptor.input_schema.clone(),
            strict: false,
            deferred: descriptor.exposure == ToolExposure::Deferred,
        });
        if let hanafubuki_agent_contracts::ToolAvailability::Freeform(freeform) = availability {
            provider_spec = ToolSpec::Freeform(freeform);
        }
        let mut planned = Self {
            provider_id: tool.provider_id,
            tool_id: tool.tool_id,
            generation: tool.generation,
            model_spec,
            provider_spec,
            namespace_description: tool.namespace_description,
            exposure: descriptor.exposure,
            binding: AgentPlannedToolBinding::Handler,
            execution_class: descriptor.execution_class,
            presentation: AgentToolPresentation {
                action: descriptor.action.clone(),
                subject: tool.subject,
            },
            memory_context_effect: descriptor.memory_effect,
            trust: AgentToolTrust::from(tool.trust),
            search_info: None,
            handler: Some(AgentToolHandlerBinding {
                handler: tool.handler.clone(),
                owner: HandlerOwner::Sourced(source),
            }),
        };
        planned.refresh_search_info();
        Ok(planned)
    }

    pub(super) fn plan_search(
        provider_id: AgentToolProviderId,
        mut model_spec: ModelToolSpec,
        execution_class: AgentToolExecutionClass,
        exposure: ToolExposure,
    ) -> Result<Self, AgentToolCatalogError> {
        model_spec.strict = false;
        let tool_id = AgentToolId::parse(model_spec.name.clone())
            .map_err(|_| AgentToolCatalogError::InvalidToolName)?;
        let provider_spec = ToolSpec::Function(FunctionToolSpec {
            name: ToolName::parse(model_spec.name.clone())
                .map_err(|_| AgentToolCatalogError::InvalidToolName)?,
            description: model_spec.description.clone(),
            input_schema: model_spec.input_schema.clone(),
            strict: false,
            deferred: false,
        });
        let canonical = serde_json::to_vec(&(
            provider_id.as_str(),
            tool_id.as_str(),
            &model_spec,
            &provider_spec,
            exposure,
            AgentPlannedToolBinding::PlanSearch,
            execution_class,
        ))
        .map_err(|_| AgentToolCatalogError::InvalidSchema)?;
        Ok(Self {
            provider_id,
            tool_id,
            generation: AgentToolGeneration(Sha256::digest(canonical).into()),
            model_spec,
            provider_spec,
            namespace_description: None,
            exposure,
            binding: AgentPlannedToolBinding::PlanSearch,
            execution_class,
            presentation: AgentToolPresentation {
                action: AgentActivityAction::USE_TOOL,
                subject: Some("tool discovery".to_string()),
            },
            memory_context_effect: AgentMemoryContextEffect::None,
            trust: AgentToolTrust::Trusted,
            search_info: None,
            handler: None,
        })
    }

    pub(super) fn refresh_search_info(&mut self) {
        self.search_info = build_search_info(self);
    }
}
