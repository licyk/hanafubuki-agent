use super::*;
use crate::{AgentKernelFaultSink, NoopAgentKernelFaultSink};

impl AgentService {
    pub fn create_private_model_session(
        protocol: ModelApiProtocol,
        base_url: &str,
        credential: Option<String>,
        headers: Vec<hanafubuki_agent_model_api::ModelHeader>,
        retry_policy: ModelRetryPolicy,
    ) -> Result<Arc<dyn ModelSession>, AgentError> {
        Ok(configured_model(
            protocol,
            base_url,
            credential,
            headers,
            retry_policy,
            hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary,
        )?
        .create_model_session())
    }

    pub fn new(model: Arc<dyn ModelPort>, registries: AgentKernelRegistries) -> Self {
        Self::with_optional_model(
            Some(model),
            registries,
            Arc::new(EphemeralAgentHistoryStore::default()),
            ApprovalService::manual(),
            FIRST_BATCH_HARD_LIMITS,
            Arc::new(NoopAgentKernelFaultSink),
        )
    }

    pub fn with_limits(
        model: Arc<dyn ModelPort>,
        registries: AgentKernelRegistries,
        limits: AgentHardLimits,
    ) -> Self {
        Self::with_optional_model(
            Some(model),
            registries,
            Arc::new(EphemeralAgentHistoryStore::default()),
            ApprovalService::manual(),
            limits,
            Arc::new(NoopAgentKernelFaultSink),
        )
    }

    pub fn with_history_store(
        model: Arc<dyn ModelPort>,
        registries: AgentKernelRegistries,
        history_store: Arc<dyn AgentHistoryStore>,
    ) -> Self {
        Self::with_optional_model(
            Some(model),
            registries,
            history_store,
            ApprovalService::manual(),
            FIRST_BATCH_HARD_LIMITS,
            Arc::new(NoopAgentKernelFaultSink),
        )
    }

    pub fn without_default_model_with_history_store(
        registries: AgentKernelRegistries,
        history_store: Arc<dyn AgentHistoryStore>,
    ) -> Self {
        Self::without_default_model_with_history_store_and_approval(
            registries,
            history_store,
            ApprovalService::manual(),
        )
    }

    pub fn without_default_model_with_history_store_and_approval(
        registries: AgentKernelRegistries,
        history_store: Arc<dyn AgentHistoryStore>,
        approvals: ApprovalService,
    ) -> Self {
        Self::without_default_model_reporting_faults(
            registries,
            history_store,
            approvals,
            Arc::new(NoopAgentKernelFaultSink),
        )
    }

    /// The application's kernel: no default model, and every panic the
    /// kernel catches in its own code is reported to `kernel_faults`.
    pub fn without_default_model_reporting_faults(
        registries: AgentKernelRegistries,
        history_store: Arc<dyn AgentHistoryStore>,
        approvals: ApprovalService,
        kernel_faults: Arc<dyn AgentKernelFaultSink>,
    ) -> Self {
        Self::with_optional_model(
            None,
            registries,
            history_store,
            approvals,
            FIRST_BATCH_HARD_LIMITS,
            kernel_faults,
        )
    }

    /// The limits this kernel was built with.
    pub fn limits(&self) -> AgentHardLimits {
        self.inner.limits
    }

    pub(crate) fn with_optional_model(
        default_model: Option<Arc<dyn ModelPort>>,
        registries: AgentKernelRegistries,
        history_store: Arc<dyn AgentHistoryStore>,
        approvals: ApprovalService,
        limits: AgentHardLimits,
        kernel_faults: Arc<dyn AgentKernelFaultSink>,
    ) -> Self {
        assert_valid_limits(limits);
        Self {
            inner: Arc::new(AgentServiceInner {
                default_model,
                registries,
                tool_search_cache: AgentToolSearchIndexCache::default(),
                tool_spec_cache: Default::default(),
                history_store: SessionStores::new(history_store),
                limits,
                sessions: SessionTable::default(),
                next_turn_id: AtomicU64::new(1),
                next_compaction_id: AtomicU64::new(1),
                image_capabilities: Mutex::new(ModelImageCapabilityCache::default()),
                model_catalog: Default::default(),
                approvals,
                pending_approvals: Mutex::new(HashMap::new()),
                pending_user_inputs: Mutex::new(HashMap::new()),
                next_user_input_request_id: AtomicU64::new(1),
                managed_turns: Mutex::new(HashMap::new()),
                managed_turns_changed: Condvar::new(),
                managed_background: Mutex::new(HashSet::new()),
                managed_background_changed: Condvar::new(),
                event_sinks: Mutex::new(HashMap::new()),
                accepting: AtomicBool::new(true),
                kernel_faults,
            }),
        }
    }

    pub async fn list_provider_models(
        &self,
        protocol: ModelApiProtocol,
        base_url: &str,
        credential: Option<String>,
        headers: Vec<hanafubuki_agent_model_api::ModelHeader>,
    ) -> Result<Vec<ModelCatalogEntry>, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let endpoint = ModelEndpoint::parse(base_url)
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        let credential = credential
            .map(ModelCredential::new)
            .transpose()
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        let provider = headers.into_iter().fold(
            ModelProvider::new(endpoint, credential),
            |provider, header| provider.with_header(header),
        );
        let catalog = ModelCatalogClient::new(provider)
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        let entries = catalog
            .list(protocol, CancellationToken::default())
            .await
            .map_err(|error| AgentError::Model(error.to_string()))?
            .entries;
        self.inner.model_catalog.record_listing(base_url, &entries);
        Ok(entries)
    }

    /// The context window of `model` at `base_url` that a session would be
    /// given, for work that samples outside a session.
    pub fn model_context_window(
        &self,
        configured: Option<u64>,
        base_url: &str,
        model: &str,
    ) -> Option<u64> {
        self.inner
            .model_catalog
            .context_window(configured, base_url, model)
    }
}

pub(super) fn configured_model(
    protocol: ModelApiProtocol,
    base_url: &str,
    credential: Option<String>,
    headers: Vec<hanafubuki_agent_model_api::ModelHeader>,
    retry_policy: ModelRetryPolicy,
    compaction_capability: hanafubuki_agent_model_api::ModelCompactionCapability,
) -> Result<Arc<dyn ModelPort>, AgentError> {
    let endpoint = ModelEndpoint::parse(base_url)
        .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
    let credential = credential
        .map(ModelCredential::new)
        .transpose()
        .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
    let provider = headers
        .into_iter()
        .fold(
            ModelProvider::new(endpoint, credential),
            |provider, header| provider.with_header(header),
        )
        .with_retry_policy(retry_policy);
    match protocol {
        ModelApiProtocol::Responses => ResponsesModelTransport::new(provider).map(|transport| {
            Arc::new(transport.with_compaction_capability(compaction_capability))
                as Arc<dyn ModelPort>
        }),
        ModelApiProtocol::ChatCompletions => ChatCompletionsModelTransport::new(provider)
            .map(|transport| Arc::new(transport) as Arc<dyn ModelPort>),
        ModelApiProtocol::AnthropicMessages => AnthropicMessagesModelTransport::new(provider)
            .map(|transport| Arc::new(transport) as Arc<dyn ModelPort>),
    }
    .map_err(|error| AgentError::InvalidInput(error.to_string()))
}
