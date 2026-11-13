use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::{
    ModelApiProtocol, ModelCatalogEntry, ModelCompactionCapability, ModelEndpoint,
    ModelEndpointKey, ModelError, ModelEvent, ModelEventStream, ModelFuture, ModelMessagePhase,
    ModelMetadataSource, ModelOutputItem, ModelPort, ModelRequest, ModelResponseMetadata,
    ModelResult, ModelRouteKey, ModelStream, ModelUsage,
};

use crate::{
    AgentHistoryStore, AgentKernelRegistries, AgentService, AgentSessionConfig, AgentSessionId,
    EphemeralAgentHistoryStore,
};

const MODEL: &str = "routed-model";
const DAY_MS: u64 = 24 * 60 * 60 * 1_000;

/// A Provider's endpoint that answers every request at once, or refuses
/// it once `refuse` is set.
#[derive(Default)]
struct RoutedModel {
    refuse: AtomicBool,
}

impl ModelPort for RoutedModel {
    fn route_key(&self, model: &str) -> Option<ModelRouteKey> {
        Some(endpoint().route(model))
    }

    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        let refuse = self.refuse.load(Ordering::SeqCst);
        Box::pin(async move {
            if refuse {
                return Err(ModelError::Remote {
                    status: Some(400),
                    code: None,
                    message: "refused".to_string(),
                    request_id: None,
                });
            }
            Ok(Box::new(Answer(answer("Done.", 1_000))) as ModelStream)
        })
    }
}

struct Answer(VecDeque<ModelResult<ModelEvent>>);

impl ModelEventStream for Answer {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        let event = self.0.pop_front();
        Box::pin(async move { event })
    }
}

/// A final answer of `text`, its request counted at `input_tokens`.
fn answer(text: &str, input_tokens: u64) -> VecDeque<ModelResult<ModelEvent>> {
    let item = |text: &str| ModelOutputItem::AssistantMessage {
        item_id: "answer".to_string(),
        phase: Some(ModelMessagePhase::FinalAnswer),
        text: text.to_string(),
    };
    VecDeque::from([
        Ok(ModelEvent::ResponseStarted {
            response_id: "response".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded { item: item("") }),
        Ok(ModelEvent::OutputItemCompleted { item: item(text) }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response".to_string(),
            usage: Some(ModelUsage {
                input_tokens,
                output_tokens: 1,
                ..Default::default()
            }),
            end_turn: Some(true),
        }),
    ])
}

fn endpoint() -> ModelEndpointKey {
    ModelEndpointKey::new(
        ModelApiProtocol::ChatCompletions,
        &ModelEndpoint::parse("https://routed.test/v1").unwrap(),
    )
}

fn listed(window: u64) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: MODEL.to_string(),
        display_name: None,
        default_reasoning_effort: None,
        supported_reasoning_efforts: Vec::new(),
        context_window: Some(window),
        max_output_tokens: None,
        image_input: None,
        reasoning: None,
    }
}

fn config(window: Option<u64>, source: Option<ModelMetadataSource>) -> AgentSessionConfig {
    AgentSessionConfig {
        provider_id: "provider-1".to_string(),
        provider_protocol: ModelApiProtocol::Responses,
        model: MODEL.to_string(),
        instructions: "test".to_string(),
        mode: crate::AgentSessionMode::Default,
        reasoning_effort: None,
        context_window_tokens: window,
        input_capabilities: Default::default(),
        auto_compaction_enabled: true,
        auto_compact_threshold_percent: 80,
        compaction_capability: ModelCompactionCapability::LocalSummary,
        sampling_retry_policy: crate::AgentSamplingRetryPolicy::default(),
        context_window_source: source,
        compaction_strategy: Default::default(),
        scope: Default::default(),
        configured_facts: Default::default(),
        role_instructions: None,
    }
}

fn service(store: &Arc<EphemeralAgentHistoryStore>) -> AgentService {
    service_over(store, Arc::new(RoutedModel::default()))
}

fn service_over(store: &Arc<EphemeralAgentHistoryStore>, model: Arc<RoutedModel>) -> AgentService {
    AgentService::with_history_store(model, AgentKernelRegistries::default(), store.clone())
}

fn record(service: &AgentService, entries: &[ModelCatalogEntry], at_ms: u64) {
    service
        .inner
        .model_metadata
        .record_listing(&endpoint(), entries, at_ms);
}

async fn turn(service: &AgentService, session_id: AgentSessionId) {
    service
        .start_turn(session_id, "hello".to_string())
        .await
        .unwrap();
}

/// The persisted window, its source, and the window its usage was
/// measured against.
fn persisted(
    store: &EphemeralAgentHistoryStore,
    session_id: AgentSessionId,
) -> (Option<u64>, Option<ModelMetadataSource>, Option<u64>) {
    let runtime = store.load_runtime_state(session_id).unwrap();
    (
        runtime.config.context_window_tokens,
        runtime.config.context_window_source,
        runtime.usage.and_then(|usage| usage.context_window_tokens),
    )
}

fn now_ms() -> u64 {
    super::now_millis_u64()
}

/// A session created before its model's window was known takes the window
/// a listing reported since, at its next turn's start, and its usage is
/// measured against it in the same commit: the turn fails before any answer
/// (so no answer's usage replaces the last one), and the session still
/// recovers.
#[tokio::test]
async fn a_session_takes_a_window_learned_after_it_was_created_at_its_next_turn() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let model = Arc::new(RoutedModel::default());
    let service = service_over(&store, model.clone());
    let session_id = service.create_session(config(None, None)).unwrap().id;
    turn(&service, session_id).await;
    assert_eq!(persisted(&store, session_id), (None, None, None));

    record(&service, &[listed(123_456)], now_ms());
    model.refuse.store(true, Ordering::SeqCst);
    assert!(service
        .start_turn(session_id, "hello".to_string())
        .await
        .is_err());

    let expected = (
        Some(123_456),
        Some(ModelMetadataSource::Reported),
        Some(123_456),
    );
    assert_eq!(persisted(&store, session_id), expected);
    drop(service);
    let recovered = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let report = recovered
        .recover_sessions_with_test_model(Arc::new(RoutedModel::default()))
        .unwrap();
    assert_eq!((report.restored, report.quarantined.len()), (1, 0));
    assert_eq!(
        recovered
            .snapshot(session_id)
            .unwrap()
            .usage
            .unwrap()
            .context_window_tokens,
        Some(123_456)
    );
}

/// A configured window is the person's: what the Provider reports never
/// replaces it.
#[tokio::test]
async fn a_configured_window_is_never_replaced() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = service(&store);
    record(&service, &[listed(123_456)], now_ms());
    let configured = config(Some(50_000), Some(ModelMetadataSource::Configured));
    let session_id = service.create_session(configured).unwrap().id;

    turn(&service, session_id).await;

    assert_eq!(
        persisted(&store, session_id),
        (
            Some(50_000),
            Some(ModelMetadataSource::Configured),
            Some(50_000)
        )
    );
}

/// A session written before sources were kept: its window counts as
/// configured and its record keeps no source; without a window it takes
/// the one known.
#[tokio::test]
async fn a_session_written_before_sources_keeps_its_window_and_takes_one_it_lacked() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = service(&store);
    record(&service, &[listed(123_456)], now_ms());
    let with_window = service
        .create_session(config(Some(50_000), None))
        .unwrap()
        .id;
    let without = service.create_session(config(None, None)).unwrap().id;

    turn(&service, with_window).await;
    turn(&service, without).await;

    assert_eq!(
        persisted(&store, with_window),
        (Some(50_000), None, Some(50_000))
    );
    assert_eq!(
        persisted(&store, without),
        (
            Some(123_456),
            Some(ModelMetadataSource::Reported),
            Some(123_456)
        )
    );
}

/// A window that is no longer known (the listing that reported it
/// expired) is kept: a known window is never dropped at a turn's start.
#[tokio::test]
async fn a_window_no_longer_known_is_kept() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = service(&store);
    let listed_at = now_ms() - 8 * DAY_MS;
    record(&service, &[listed(123_456)], listed_at);
    let session_id = service.create_session(config(None, None)).unwrap().id;
    turn(&service, session_id).await;
    let reported = (
        Some(123_456),
        Some(ModelMetadataSource::Reported),
        Some(123_456),
    );
    assert_eq!(persisted(&store, session_id), reported);

    // Listed no more (the second empty listing passes the shrink guard),
    // the model keeps what it reported 7 days only.
    record(&service, &[], listed_at + 1);
    record(&service, &[], listed_at + 2);
    assert_eq!(
        service.inner.model_metadata.context_window(
            None,
            Some(&endpoint().route(MODEL)),
            MODEL,
            now_ms()
        ),
        None
    );
    turn(&service, session_id).await;

    assert_eq!(persisted(&store, session_id), reported);
}

/// Creation and a model selection record where the window came from: the
/// policy's window is configured, the built-in table's is built-in.
#[test]
fn creation_and_model_selection_record_the_windows_source() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let create = |window: Option<u64>| {
        let mut config = config(window, None);
        config.model = "gpt-5.1".to_string();
        service
            .create_model_session(
                config,
                ModelApiProtocol::Responses,
                "http://127.0.0.1:1/v1",
                None,
                Vec::new(),
                crate::ModelRetryPolicy::default(),
                ModelCompactionCapability::LocalSummary,
                crate::AgentSessionWorkspace::default(),
            )
            .unwrap()
    };
    let configured = create(Some(50_000));
    let built_in = create(None);
    assert_eq!(
        persisted(&store, configured.id),
        (Some(50_000), Some(ModelMetadataSource::Configured), None)
    );
    assert_eq!(
        persisted(&store, built_in.id),
        (Some(272_000), Some(ModelMetadataSource::BuiltIn), None)
    );

    let mut update = crate::AgentSessionModelConfigurationUpdate {
        provider_id: "provider-1".to_string(),
        provider_protocol: ModelApiProtocol::Responses,
        model: "unknown-model".to_string(),
        instructions: "test".to_string(),
        reasoning_effort: None,
        configured_facts: Default::default(),
        input_capabilities: Default::default(),
        auto_compaction_enabled: true,
        auto_compact_threshold_percent: 80,
        compaction_capability: ModelCompactionCapability::LocalSummary,
        turn_budget: Default::default(),
    };
    let select = |session: &crate::AgentSessionRuntimeSnapshot,
                  update: crate::AgentSessionModelConfigurationUpdate| {
        service
            .set_session_model_configuration(
                session.id,
                session.generation,
                session.revision,
                ModelApiProtocol::Responses,
                "http://127.0.0.1:1/v1",
                None,
                Vec::new(),
                crate::ModelRetryPolicy::default(),
                update,
            )
            .unwrap()
    };
    select(&configured, update.clone());
    assert_eq!(persisted(&store, configured.id), (None, None, None));
    update.configured_facts.context_window_tokens = Some(64_000);
    select(&built_in, update);
    assert_eq!(
        persisted(&store, built_in.id),
        (Some(64_000), Some(ModelMetadataSource::Configured), None)
    );
}

mod configured;
mod learned;
mod listed;
