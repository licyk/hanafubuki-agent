//! Windows learned from a Provider's refusal of a request as too long: the
//! route keeps the smallest, the refused session takes it at once (before
//! its compaction and retry) unless its window is configured, and later
//! sessions, in this run and the next, start with it. And the room the
//! threshold keeps for a reply, from the route's largest output.

use std::collections::VecDeque;
use std::sync::Arc;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::{
    ModelCatalogEntry, ModelError, ModelFuture, ModelMetadataSource, ModelPort, ModelRequest,
    ModelResult, ModelRouteKey, ModelStream,
};
use parking_lot::Mutex;
use serde_json::json;

use super::{answer, config, endpoint, listed, now_ms, persisted, record, Answer, MODEL};
use crate::model_metadata::{ModelMetadataSnapshot, ModelMetadataStore};
use crate::service::estimate_model_request_tokens;
use crate::{
    AgentCompactionPhase, AgentCompactionReason, AgentCompactionTrigger, AgentError, AgentEvent,
    AgentEventKind, AgentEventSink, AgentKernelRegistries, AgentService, AgentSessionId,
    EphemeralAgentHistoryStore, NoopAgentKernelFaultSink,
};

const NAMED: &str = "This model's maximum context length is 50000 tokens. However, your messages resulted in 61000 tokens.";
const UNNAMED: &str = "Input is too long for requested model.";

/// What the scripted endpoint answers a request with.
enum Reply {
    /// A refusal as too long, with this message.
    TooLong(&'static str),
    /// An answer of this text, its request counted at this many tokens.
    Answer(String, u64),
}

fn long_answer(input_tokens: u64) -> Reply {
    Reply::Answer("old context ".repeat(1_000), input_tokens)
}

fn done(input_tokens: u64) -> Reply {
    Reply::Answer("Done.".to_string(), input_tokens)
}

fn summary() -> Reply {
    Reply::Answer("The user said hello.".to_string(), 100)
}

/// The Provider's endpoint at the test route: it answers each request,
/// a compaction's included, with the next of its replies, and keeps them.
struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl Scripted {
    fn new(replies: impl IntoIterator<Item = Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().clone()
    }
}

impl ModelPort for Scripted {
    fn route_key(&self, model: &str) -> Option<ModelRouteKey> {
        Some(endpoint().route(model))
    }

    fn stream(
        &self,
        request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.requests.lock().push(request);
        let reply = self.replies.lock().pop_front();
        Box::pin(async move {
            match reply.ok_or_else(|| ModelError::Protocol("no reply left".to_string()))? {
                Reply::TooLong(message) => Err(ModelError::ContextWindowExceeded {
                    status: Some(400),
                    code: None,
                    message: message.to_string(),
                    request_id: None,
                }),
                Reply::Answer(text, input_tokens) => {
                    Ok(Box::new(Answer(answer(&text, input_tokens))) as ModelStream)
                }
            }
        })
    }
}

/// Keeps the snapshot it was last given, and loads it.
#[derive(Default)]
struct SharedStore(Mutex<ModelMetadataSnapshot>);

impl ModelMetadataStore for SharedStore {
    fn load(&self) -> ModelMetadataSnapshot {
        self.0.lock().clone()
    }

    fn save(&self, snapshot: ModelMetadataSnapshot) {
        *self.0.lock() = snapshot;
    }
}

impl SharedStore {
    /// The window the route learned, as the store keeps it.
    fn learned_window(&self) -> Option<u64> {
        let snapshot = serde_json::to_value(&*self.0.lock()).unwrap();
        snapshot["routes"][0]["learned"]["contextWindow"]["value"].as_u64()
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<AgentEvent>>);

impl AgentEventSink for RecordingSink {
    fn emit(&self, event: AgentEvent) {
        self.0.lock().push(event);
    }
}

impl RecordingSink {
    fn learned(&self) -> Vec<AgentEventKind> {
        self.0
            .lock()
            .iter()
            .filter(|event| matches!(event.kind, AgentEventKind::ModelContextWindowLearned { .. }))
            .map(|event| event.kind.clone())
            .collect()
    }
}

struct Fixture {
    service: AgentService,
    history: Arc<EphemeralAgentHistoryStore>,
    model: Arc<Scripted>,
    metadata: Arc<SharedStore>,
}

impl Fixture {
    fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
        Self::over(Arc::new(SharedStore::default()), replies)
    }

    /// A kernel whose model metadata is kept in `metadata`.
    fn over(metadata: Arc<SharedStore>, replies: impl IntoIterator<Item = Reply>) -> Self {
        let history = Arc::new(EphemeralAgentHistoryStore::default());
        let model = Scripted::new(replies);
        let service = AgentService::with_optional_model(
            Some(model.clone()),
            AgentKernelRegistries::default(),
            history.clone(),
            hanafubuki_agent_approval::ApprovalService::manual(),
            crate::limits::FIRST_BATCH_HARD_LIMITS,
            Arc::new(NoopAgentKernelFaultSink),
            metadata.clone(),
        );
        Self {
            service,
            history,
            model,
            metadata,
        }
    }

    fn session(&self, window: Option<u64>, source: Option<ModelMetadataSource>) -> AgentSessionId {
        self.service
            .create_session(config(window, source))
            .unwrap()
            .id
    }

    async fn turn(
        &self,
        session_id: AgentSessionId,
        text: &str,
    ) -> (Result<String, AgentError>, Arc<RecordingSink>) {
        let sink = Arc::new(RecordingSink::default());
        let outcome = self
            .service
            .start_turn_with_sink(session_id, text.to_string(), sink.clone())
            .await
            .map(|outcome| outcome.final_answer);
        (outcome, sink)
    }

    /// The window the route resolves to, and its source.
    fn route_window(&self) -> Option<(u64, ModelMetadataSource)> {
        self.service
            .inner
            .model_metadata
            .context_window(None, Some(&endpoint().route(MODEL)), MODEL, now_ms())
            .map(|window| (window.value, window.source))
    }
}

fn learned(tokens: u64, previous: Option<u64>) -> AgentEventKind {
    AgentEventKind::ModelContextWindowLearned { tokens, previous }
}

/// A session with no known window that the Provider refuses, naming its
/// window: the session takes the window at once and says so, compacts,
/// retries, and from then on compacts before the window fills, at the
/// threshold's share of it, with no refusal.
#[tokio::test]
async fn a_refusal_naming_the_window_teaches_it_and_the_session_compacts_by_it_from_then_on() {
    let fixture = Fixture::new([
        long_answer(41_000),
        Reply::TooLong(NAMED),
        summary(),
        done(41_000),
        summary(),
        done(1_000),
    ]);
    let session_id = fixture.session(None, None);
    // No window known: a context of 41,000 tokens does not compact.
    assert!(fixture.turn(session_id, "hello").await.0.is_ok());

    let (outcome, sink) = fixture.turn(session_id, "go on").await;

    assert_eq!(outcome.unwrap(), "Done.");
    assert_eq!(sink.learned(), vec![learned(50_000, None)]);
    let events = sink.0.lock().clone();
    let learned_at = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::ModelContextWindowLearned { .. }))
        .unwrap();
    let compacted_at = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::CompactionUpdated { .. }))
        .unwrap();
    assert!(learned_at < compacted_at);
    let learned_window = (
        Some(50_000),
        Some(ModelMetadataSource::Learned),
        Some(50_000),
    );
    assert_eq!(persisted(&fixture.history, session_id), learned_window);
    assert_eq!(
        fixture.route_window(),
        Some((50_000, ModelMetadataSource::Learned))
    );
    assert_eq!(fixture.metadata.learned_window(), Some(50_000));
    assert_eq!(fixture.model.requests().len(), 4);

    // 41,000 tokens reach 80% of 50,000: the next turn compacts first.
    let (outcome, sink) = fixture.turn(session_id, "and then?").await;

    assert_eq!(outcome.unwrap(), "Done.");
    assert!(sink.learned().is_empty());
    assert_eq!(fixture.model.requests().len(), 6);
    let snapshot = fixture.service.snapshot(session_id).unwrap();
    let compaction = snapshot.compaction.unwrap();
    assert_eq!(compaction.trigger, AgentCompactionTrigger::Automatic);
    assert_eq!(compaction.reason, AgentCompactionReason::ContextLimit);
    assert_eq!(compaction.phase, AgentCompactionPhase::PreTurn);
    assert_eq!(snapshot.context_window.window_number, 2);
    assert_eq!(persisted(&fixture.history, session_id), learned_window);
}

/// A refusal that names no window teaches the refused request's estimated
/// size, an upper bound of the window.
#[tokio::test]
async fn a_refusal_naming_no_window_teaches_the_refused_requests_size() {
    let fixture = Fixture::new([
        long_answer(1_000),
        Reply::TooLong(UNNAMED),
        summary(),
        done(1_000),
    ]);
    let session_id = fixture.session(None, None);
    assert!(fixture.turn(session_id, "hello").await.0.is_ok());

    let (outcome, sink) = fixture.turn(session_id, "go on").await;

    assert_eq!(outcome.unwrap(), "Done.");
    let refused = &fixture.model.requests()[1];
    let estimated = estimate_model_request_tokens(refused).unwrap();
    assert!(estimated > 1_024, "{estimated}");
    assert_eq!(sink.learned(), vec![learned(estimated, None)]);
    assert_eq!(
        persisted(&fixture.history, session_id),
        (
            Some(estimated),
            Some(ModelMetadataSource::Learned),
            Some(estimated)
        )
    );
}

/// A learned window is used only while it is smaller than the reported
/// one: the route learns it, but the session keeps the reported window
/// and nothing is said.
#[tokio::test]
async fn a_window_learned_larger_than_the_reported_one_is_not_used() {
    let fixture = Fixture::new([
        long_answer(1_000),
        Reply::TooLong("maximum context length is 100000 tokens"),
        summary(),
        done(1_000),
    ]);
    record(&fixture.service, &[listed(64_000)], now_ms());
    let session_id = fixture.session(None, None);
    assert!(fixture.turn(session_id, "hello").await.0.is_ok());

    let (outcome, sink) = fixture.turn(session_id, "go on").await;

    assert_eq!(outcome.unwrap(), "Done.");
    assert!(sink.learned().is_empty());
    assert_eq!(fixture.metadata.learned_window(), Some(100_000));
    assert_eq!(
        fixture.route_window(),
        Some((64_000, ModelMetadataSource::Reported))
    );
    assert_eq!(
        persisted(&fixture.history, session_id),
        (
            Some(64_000),
            Some(ModelMetadataSource::Reported),
            Some(64_000)
        )
    );
}

/// A configured window is the person's: the refused session keeps it and
/// says nothing, but the route learns, and a session on it with no
/// configured window starts with what it learned.
#[tokio::test]
async fn a_configured_session_keeps_its_window_while_the_route_learns() {
    let fixture = Fixture::new([
        long_answer(1_000),
        Reply::TooLong(NAMED),
        summary(),
        done(1_000),
        done(1_000),
    ]);
    let configured = fixture.session(Some(300_000), Some(ModelMetadataSource::Configured));
    assert!(fixture.turn(configured, "hello").await.0.is_ok());

    let (outcome, sink) = fixture.turn(configured, "go on").await;

    assert_eq!(outcome.unwrap(), "Done.");
    assert!(sink.learned().is_empty());
    assert_eq!(
        persisted(&fixture.history, configured),
        (
            Some(300_000),
            Some(ModelMetadataSource::Configured),
            Some(300_000)
        )
    );
    assert_eq!(
        fixture.route_window(),
        Some((50_000, ModelMetadataSource::Learned))
    );
    let other = fixture.session(None, None);
    assert!(fixture.turn(other, "hello").await.0.is_ok());
    assert_eq!(
        persisted(&fixture.history, other).1,
        Some(ModelMetadataSource::Learned)
    );
    assert_eq!(persisted(&fixture.history, other).0, Some(50_000));
}

/// With automatic compaction off, a refusal ends the turn as before, but
/// the window is learned all the same: the session takes it, the store
/// keeps it, and a session of the next run starts with it.
#[tokio::test]
async fn a_window_is_learned_without_automatic_compaction_and_outlives_the_run() {
    let metadata = Arc::new(SharedStore::default());
    let fixture = Fixture::over(
        metadata.clone(),
        [long_answer(1_000), Reply::TooLong(NAMED)],
    );
    let mut config = config(None, None);
    config.auto_compaction_enabled = false;
    let session_id = fixture.service.create_session(config).unwrap().id;
    assert!(fixture.turn(session_id, "hello").await.0.is_ok());

    let (outcome, sink) = fixture.turn(session_id, "go on").await;

    assert!(matches!(
        outcome,
        Err(AgentError::ProviderContextWindowExceeded { .. })
    ));
    assert_eq!(fixture.model.requests().len(), 2);
    assert_eq!(sink.learned(), vec![learned(50_000, None)]);
    assert_eq!(persisted(&fixture.history, session_id).0, Some(50_000));
    assert_eq!(metadata.learned_window(), Some(50_000));
    // The window was rewritten during the turn, with its usage's: the
    // session still recovers.
    let Fixture {
        service, history, ..
    } = fixture;
    drop(service);
    let recovered = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        history,
    );
    let report = recovered
        .recover_sessions_with_test_model(Scripted::new([]))
        .unwrap();
    assert_eq!((report.restored, report.quarantined.len()), (1, 0));
    let usage = recovered.snapshot(session_id).unwrap().usage.unwrap();
    assert_eq!(usage.context_window_tokens, Some(50_000));

    let next_run = Fixture::over(metadata, [done(1_000)]);
    let session_id = next_run.session(None, None);
    assert!(next_run.turn(session_id, "hello").await.0.is_ok());
    assert_eq!(
        persisted(&next_run.history, session_id),
        (
            Some(50_000),
            Some(ModelMetadataSource::Learned),
            Some(50_000)
        )
    );
}

/// The threshold keeps room for the route's largest output: listed at
/// 1,000 tokens on a window of 20,000, a context of 15,950 stays under the
/// threshold of 16,000, where the 32,768 assumed of an unknown output would
/// have kept 4,096 and compacted at 15,904.
#[tokio::test]
async fn the_threshold_keeps_room_for_the_routes_largest_output() {
    let fixture = Fixture::new([done(15_950), done(1_000)]);
    let entry = ModelCatalogEntry {
        max_output_tokens: Some(1_000),
        ..listed(20_000)
    };
    record(&fixture.service, &[entry], now_ms());
    let session_id = fixture.session(None, None);
    assert!(fixture.turn(session_id, "hello").await.0.is_ok());

    assert!(fixture.turn(session_id, "go on").await.0.is_ok());

    assert_eq!(fixture.model.requests().len(), 2);
    assert!(fixture
        .service
        .snapshot(session_id)
        .unwrap()
        .compaction
        .is_none());
}

#[test]
fn the_event_serializes_with_its_previous_window_null_when_unknown() {
    assert_eq!(
        serde_json::to_value(learned(50_000, None)).unwrap(),
        json!({"type": "model_context_window_learned", "tokens": 50_000, "previous": null})
    );
    assert_eq!(
        serde_json::to_value(learned(50_000, Some(64_000))).unwrap(),
        json!({"type": "model_context_window_learned", "tokens": 50_000, "previous": 64_000})
    );
}
