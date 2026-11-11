//! What the person configures for a session's model, and forgetting what
//! the Provider's refusals taught: a session takes either at its next
//! turn's admission.

use std::sync::Arc;

use hanafubuki_agent_model_api::{ModelApiProtocol, ModelMetadataSource};

use super::{config, endpoint, listed, now_ms, persisted, record, service, turn, MODEL};
use crate::{AgentService, AgentSessionId, EphemeralAgentHistoryStore};

const BASE_URL: &str = "https://routed.test/v1";

fn learn(service: &AgentService, tokens: u64) {
    service
        .inner
        .model_metadata
        .learn_context_window(&endpoint().route(MODEL), tokens, now_ms());
}

fn forget(service: &AgentService) -> usize {
    service
        .forget_learned_model_facts(ModelApiProtocol::ChatCompletions, BASE_URL, Some(MODEL))
        .unwrap()
}

/// What the person configured for the session's model, as an activation
/// gives it the configuration it reads now.
fn configure(service: &AgentService, session_id: AgentSessionId, window: Option<u64>) {
    let cell = service.session(session_id).unwrap();
    let mut record = cell.lock().unwrap();
    record
        .settings
        .config
        .configured_facts
        .context_window_tokens = window;
}

/// A window learned when nothing else was known limits the session; once
/// forgotten, the next turn's admission leaves the window unknown (the
/// session compacts on the Provider's error again), and when a listing
/// reported one, the session takes that.
#[tokio::test]
async fn a_forgotten_learned_window_no_longer_limits_the_next_turn() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = service(&store);
    let session_id = service.create_session(config(None, None)).unwrap().id;
    learn(&service, 40_000);
    turn(&service, session_id).await;
    let learned = (
        Some(40_000),
        Some(ModelMetadataSource::Learned),
        Some(40_000),
    );
    assert_eq!(persisted(&store, session_id), learned);

    assert_eq!(forget(&service), 1);
    turn(&service, session_id).await;
    assert_eq!(persisted(&store, session_id), (None, None, None));
    assert_eq!(
        service
            .session_model_metadata(session_id)
            .unwrap()
            .context_window,
        None
    );

    record(&service, &[listed(123_456)], now_ms());
    learn(&service, 40_000);
    turn(&service, session_id).await;
    assert_eq!(persisted(&store, session_id), learned);
    assert_eq!(forget(&service), 1);
    turn(&service, session_id).await;
    assert_eq!(
        persisted(&store, session_id),
        (
            Some(123_456),
            Some(ModelMetadataSource::Reported),
            Some(123_456)
        )
    );
    assert_eq!(forget(&service), 0, "nothing left to forget");
}

/// A window configured for the model beats a smaller learned one and a
/// reported one, and when the configuration no longer names one, the
/// session takes what is known again.
#[tokio::test]
async fn a_configured_window_is_taken_and_given_up_at_the_next_turn() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = service(&store);
    record(&service, &[listed(123_456)], now_ms());
    learn(&service, 40_000);
    let session_id = service.create_session(config(None, None)).unwrap().id;
    turn(&service, session_id).await;
    assert_eq!(
        persisted(&store, session_id).1,
        Some(ModelMetadataSource::Learned)
    );

    configure(&service, session_id, Some(200_000));
    turn(&service, session_id).await;
    assert_eq!(
        persisted(&store, session_id),
        (
            Some(200_000),
            Some(ModelMetadataSource::Configured),
            Some(200_000)
        )
    );
    let metadata = service.session_model_metadata(session_id).unwrap();
    assert_eq!(
        metadata
            .context_window
            .map(|window| (window.value, window.source)),
        Some((200_000, ModelMetadataSource::Configured))
    );

    configure(&service, session_id, None);
    turn(&service, session_id).await;
    assert_eq!(
        persisted(&store, session_id),
        (
            Some(40_000),
            Some(ModelMetadataSource::Learned),
            Some(40_000)
        )
    );
}

/// A window written without a source (before sources were kept), whose
/// configuration no longer names one, is kept while nothing else is known,
/// and gives way to a reported window.
#[tokio::test]
async fn a_window_written_without_a_source_is_kept_until_another_is_known() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = service(&store);
    let session_id = service
        .create_session(config(Some(50_000), None))
        .unwrap()
        .id;
    configure(&service, session_id, None);

    turn(&service, session_id).await;
    assert_eq!(
        persisted(&store, session_id),
        (Some(50_000), None, Some(50_000))
    );

    record(&service, &[listed(123_456)], now_ms());
    turn(&service, session_id).await;
    assert_eq!(
        persisted(&store, session_id),
        (
            Some(123_456),
            Some(ModelMetadataSource::Reported),
            Some(123_456)
        )
    );
}
