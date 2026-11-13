//! A listing recorded after a session was created reaches it without a
//! turn: at once when nothing is staged, else at its next response's
//! commit. `/status`'s window (`session_model_metadata`) and the window
//! compaction measures against (the session's config) are then the listed
//! one, and the usage is measured against it.

use std::sync::{Arc, OnceLock};

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::{
    ModelFuture, ModelMetadataSource, ModelPort, ModelRequest, ModelResult, ModelRouteKey,
    ModelStream,
};

use super::{answer, config, endpoint, listed, now_ms, persisted, record, service, turn, Answer};
use crate::model_metadata::ModelMetadataRegistry;
use crate::{AgentKernelRegistries, AgentService, AgentSessionId, EphemeralAgentHistoryStore};

const REPORTED: (Option<u64>, Option<ModelMetadataSource>) =
    (Some(123_456), Some(ModelMetadataSource::Reported));

/// Records a listing of the test endpoint reporting `window`, as
/// `list_provider_models` does after the Provider answered.
async fn list(service: &AgentService, window: u64) {
    record(service, &[listed(window)], now_ms());
    service.take_listed_context_windows(&endpoint()).await;
}

/// The window `/status` shows, and its source.
fn status_window(
    service: &AgentService,
    session_id: AgentSessionId,
) -> Option<(u64, ModelMetadataSource)> {
    service
        .session_model_metadata(session_id)
        .unwrap()
        .context_window
        .map(|window| (window.value, window.source))
}

/// The window the session's compaction threshold is measured against.
fn compaction_window(service: &AgentService, session_id: AgentSessionId) -> Option<u64> {
    service
        .session(session_id)
        .unwrap()
        .lock()
        .unwrap()
        .settings
        .config
        .context_window_tokens
}

/// When the session last changed, which orders the session list.
fn updated_at(service: &AgentService, session_id: AgentSessionId) -> u128 {
    service
        .session(session_id)
        .unwrap()
        .lock()
        .unwrap()
        .status
        .updated_at
}

#[tokio::test]
async fn a_listing_after_creation_reaches_the_session_before_any_turn() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = service(&store);
    let session = service.create_session(config(None, None)).unwrap();
    let configured = service
        .create_session(config(Some(50_000), Some(ModelMetadataSource::Configured)))
        .unwrap();
    let mut other_model = config(None, None);
    other_model.model = "unlisted-model".to_string();
    let unlisted = service.create_session(other_model).unwrap();
    assert_eq!(status_window(&service, session.id), None);
    let last_updated = updated_at(&service, session.id);

    list(&service, 123_456).await;

    assert_eq!(
        status_window(&service, session.id),
        Some((123_456, ModelMetadataSource::Reported))
    );
    assert_eq!(compaction_window(&service, session.id), Some(123_456));
    assert_eq!(
        persisted(&store, session.id),
        (REPORTED.0, REPORTED.1, None)
    );
    let snapshot = service.snapshot(session.id).unwrap();
    assert_eq!(snapshot.revision.value(), session.revision.value() + 1);
    assert_eq!(updated_at(&service, session.id), last_updated);
    // A configured window stays, and a model the listing does not name
    // takes nothing: neither commits.
    assert_eq!(compaction_window(&service, configured.id), Some(50_000));
    assert_eq!(
        service.snapshot(configured.id).unwrap().revision,
        configured.revision
    );
    assert_eq!(compaction_window(&service, unlisted.id), None);
    assert_eq!(
        service.snapshot(unlisted.id).unwrap().revision,
        unlisted.revision
    );
}

/// A session that ran a turn before its window was known has its usage
/// measured against the listed window as soon as it is taken, and still
/// recovers.
#[tokio::test]
async fn a_listed_window_keeps_the_usage_in_step() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = service(&store);
    let session_id = service.create_session(config(None, None)).unwrap().id;
    turn(&service, session_id).await;
    assert_eq!(persisted(&store, session_id), (None, None, None));

    list(&service, 123_456).await;

    let usage = service.snapshot(session_id).unwrap().usage.unwrap();
    assert_eq!(usage.context_window_tokens, Some(123_456));
    assert_eq!(
        persisted(&store, session_id),
        (REPORTED.0, REPORTED.1, Some(123_456))
    );
    assert_recovers(service, &store, session_id);
}

/// The Provider's endpoint, whose listing lands while it answers: the
/// listing is recorded in the model metadata, but no session takes it
/// then, as when the session had a commit staged.
#[derive(Default)]
struct ListedWhileAnswering {
    registry: OnceLock<Arc<ModelMetadataRegistry>>,
}

impl ModelPort for ListedWhileAnswering {
    fn route_key(&self, model: &str) -> Option<ModelRouteKey> {
        Some(endpoint().route(model))
    }

    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.registry
            .get()
            .unwrap()
            .record_listing(&endpoint(), &[listed(123_456)], now_ms());
        Box::pin(async { Ok(Box::new(Answer(answer("Done.", 1_000))) as ModelStream) })
    }
}

#[tokio::test]
async fn a_listing_that_lands_during_the_first_turn_is_taken_at_the_response_commit() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let model = Arc::new(ListedWhileAnswering::default());
    let service = AgentService::with_history_store(
        model.clone(),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let _ = model.registry.set(service.inner.model_metadata.clone());
    let session_id = service.create_session(config(None, None)).unwrap().id;

    turn(&service, session_id).await;

    assert_eq!(compaction_window(&service, session_id), Some(123_456));
    let usage = service.snapshot(session_id).unwrap().usage.unwrap();
    assert_eq!(usage.context_window_tokens, Some(123_456));
    assert_eq!(
        persisted(&store, session_id),
        (REPORTED.0, REPORTED.1, Some(123_456))
    );
    assert_recovers(service, &store, session_id);
}

/// A session not bound to its Provider (recovered, its transport
/// unavailable) has no route; with no window of its own, `/status` shows
/// the one its model resolves to without one.
#[tokio::test]
async fn a_session_with_no_route_shows_the_resolved_window() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let mut built_in = config(None, None);
    built_in.model = "gpt-5.1".to_string();
    let session_id = service(&store).create_session(built_in).unwrap().id;
    assert_eq!(persisted(&store, session_id), (None, None, None));

    let recovered = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    recovered.recover_sessions().unwrap();

    assert_eq!(
        status_window(&recovered, session_id),
        Some((272_000, ModelMetadataSource::BuiltIn))
    );
    assert_eq!(compaction_window(&recovered, session_id), None);
}

/// Drops `service` and recovers `session_id` from `store`: its usage
/// matches its window, so it is restored, not quarantined.
fn assert_recovers(
    service: AgentService,
    store: &Arc<EphemeralAgentHistoryStore>,
    session_id: AgentSessionId,
) {
    drop(service);
    let recovered = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let report = recovered.recover_sessions().unwrap();
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
