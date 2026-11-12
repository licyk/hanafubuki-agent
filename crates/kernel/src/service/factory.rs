use super::*;
use crate::model_metadata::{ModelMetadataRegistry, ModelMetadataStore, NoopModelMetadataStore};
use crate::{AgentKernelFaultSink, NoopAgentKernelFaultSink};

impl AgentService {
    /// A model session outside any session's conversation, through
    /// `proxy` (the kernel's [`Self::model_proxy`] when one is at hand).
    pub fn create_private_model_session(
        protocol: ModelApiProtocol,
        base_url: &str,
        credential: Option<String>,
        headers: Vec<hanafubuki_agent_model_api::ModelHeader>,
        retry_policy: ModelRetryPolicy,
        proxy: hanafubuki_agent_model_api::ModelHttpProxy,
    ) -> Result<Arc<dyn ModelSession>, AgentError> {
        Ok(configured_model(
            protocol,
            base_url,
            credential,
            headers,
            retry_policy,
            hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary,
            proxy,
        )?
        .create_model_session())
    }

    /// The proxy the Providers' models are built with: a session's next
    /// turn, each listing and each private request take it.
    pub fn set_model_proxy(&self, proxy: hanafubuki_agent_model_api::ModelHttpProxy) {
        *self.inner.model_proxy.write() = proxy;
    }

    pub fn model_proxy(&self) -> hanafubuki_agent_model_api::ModelHttpProxy {
        self.inner.model_proxy.read().clone()
    }

    pub fn new(model: Arc<dyn ModelPort>, registries: AgentKernelRegistries) -> Self {
        Self::with_optional_model(
            Some(model),
            registries,
            Arc::new(EphemeralAgentHistoryStore::default()),
            ApprovalService::manual(),
            FIRST_BATCH_HARD_LIMITS,
            Arc::new(NoopAgentKernelFaultSink),
            Arc::new(NoopModelMetadataStore),
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
            Arc::new(NoopModelMetadataStore),
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
            Arc::new(NoopModelMetadataStore),
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
            Arc::new(NoopModelMetadataStore),
        )
    }

    /// The application's kernel: no default model, every panic the kernel
    /// catches in its own code reported to `kernel_faults`, and what it
    /// learns about models kept in `model_metadata`.
    pub fn without_default_model_reporting_faults(
        registries: AgentKernelRegistries,
        history_store: Arc<dyn AgentHistoryStore>,
        approvals: ApprovalService,
        kernel_faults: Arc<dyn AgentKernelFaultSink>,
        model_metadata: Arc<dyn ModelMetadataStore>,
    ) -> Self {
        Self::with_optional_model(
            None,
            registries,
            history_store,
            approvals,
            FIRST_BATCH_HARD_LIMITS,
            kernel_faults,
            model_metadata,
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
        model_metadata: Arc<dyn ModelMetadataStore>,
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
                model_metadata: ModelMetadataRegistry::open(model_metadata, now_millis_u64()),
                model_proxy: Default::default(),
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

    /// Lists the models of the Provider at `base_url`, and records the
    /// listing in the model metadata. Cancelled through `cancellation`, it
    /// records nothing.
    pub async fn list_provider_models(
        &self,
        protocol: ModelApiProtocol,
        base_url: &str,
        credential: Option<String>,
        headers: Vec<hanafubuki_agent_model_api::ModelHeader>,
        cancellation: CancellationToken,
    ) -> Result<Vec<ModelCatalogEntry>, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let endpoint = ModelEndpoint::parse(base_url)
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        let endpoint_key = ModelEndpointKey::new(protocol, &endpoint);
        let credential = credential
            .map(ModelCredential::new)
            .transpose()
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        let provider = headers.into_iter().fold(
            ModelProvider::new(endpoint, credential).with_proxy(self.model_proxy()),
            |provider, header| provider.with_header(header),
        );
        let catalog = ModelCatalogClient::new(provider)
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        let entries = catalog
            .list(protocol, cancellation)
            .await
            .map_err(|error| AgentError::Model(error.to_string()))?
            .entries;
        self.inner
            .model_metadata
            .record_listing(&endpoint_key, &entries, now_millis_u64());
        Ok(entries)
    }

    /// The context window of `model` at the Provider `provider_id` at
    /// `base_url` that a session would be given, for work that samples
    /// outside a session.
    pub fn model_context_window(
        &self,
        configured: Option<u64>,
        provider_id: &str,
        protocol: ModelApiProtocol,
        base_url: &str,
        model: &str,
    ) -> Option<u64> {
        let route = ModelEndpoint::parse(base_url).ok().map(|endpoint| {
            self.name_model_provider(provider_id, protocol, &endpoint);
            ModelRouteKey::new(protocol, &endpoint, model)
        });
        self.inner
            .model_metadata
            .context_window(configured, route.as_ref(), model, now_millis_u64())
            .map(|window| window.value)
    }

    /// When the latest listing of the Provider at `base_url` was recorded,
    /// in milliseconds since the Unix epoch: none when it never was, or
    /// when `base_url` is not an endpoint.
    pub fn model_listing_recorded_at(
        &self,
        protocol: ModelApiProtocol,
        base_url: &str,
    ) -> Option<u64> {
        let endpoint = ModelEndpoint::parse(base_url).ok()?;
        self.inner
            .model_metadata
            .listed_at_ms(&ModelEndpointKey::new(protocol, &endpoint))
    }

    /// The context window a session on `model_id` over `model` is given,
    /// and its source.
    pub(super) fn session_context_window(
        &self,
        configured: Option<u64>,
        model: &dyn ModelPort,
        model_id: &str,
    ) -> Option<hanafubuki_agent_model_api::Sourced<u64>> {
        self.inner.model_metadata.context_window(
            configured,
            model.route_key(model_id).as_ref(),
            model_id,
            now_millis_u64(),
        )
    }
}

pub(super) fn configured_model(
    protocol: ModelApiProtocol,
    base_url: &str,
    credential: Option<String>,
    headers: Vec<hanafubuki_agent_model_api::ModelHeader>,
    retry_policy: ModelRetryPolicy,
    compaction_capability: hanafubuki_agent_model_api::ModelCompactionCapability,
    proxy: hanafubuki_agent_model_api::ModelHttpProxy,
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
        .with_retry_policy(retry_policy)
        .with_proxy(proxy);
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
