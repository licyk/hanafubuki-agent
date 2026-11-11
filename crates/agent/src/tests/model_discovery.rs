//! Model discovery against a fake Provider that answers `GET /models` apart
//! from the turns' event streams: the Agent lists the default Provider at
//! start, in the background, a session takes what the listing reported at
//! its next turn, and the listing is rate-limited, turned off by
//! `modelMetadata.discovery = "off"` and cancelled at shutdown.

use hanafubuki_agent_kernel::{AgentSessionMode, ModelApiProtocol};
use hanafubuki_agent_process::executor::ExecutorManager;

use super::*;
use crate::model_discovery::ModelDiscoveryPolicy;

const LISTING: &str = r#"{"data":[{"id":"fixture-model","context_length":123456}]}"#;
const HOUR: Duration = Duration::from_secs(60 * 60);

/// The Agent over `paths` and `config`, discovering under `policy`.
fn open(
    paths: &TestPaths,
    config: &AgentConfig,
    policy: ModelDiscoveryPolicy,
) -> AgentApplicationService {
    AgentApplicationService::new_with_host_discovering(
        config.clone(),
        test_host(paths),
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
        policy,
    )
    .unwrap()
}

/// The paths and configuration of an Agent whose default Provider is at
/// `base_url`, with discovery on (the test provider turns it off).
fn configured(name: &str, base_url: &str) -> (TestPaths, AgentConfig) {
    let paths = test_paths(name);
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, base_url, None);
    config
        .set_user_value("modelMetadata.discovery", "auto")
        .unwrap();
    (paths, config)
}

async fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + TEST_WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Waits until the discovery has handled `count` requests, listed or not.
async fn handled(service: &AgentApplicationService, count: u64) {
    wait_until(&format!("{count} discovery requests handled"), || {
        service.test_model_discovery().handled() >= count
    })
    .await;
}

fn recorded(service: &AgentApplicationService, server: &FakeServer) -> bool {
    service
        .core
        .model_listing_recorded_at(ModelApiProtocol::Responses, &server.base_url)
        .is_some()
}

fn stop(service: &AgentApplicationService) {
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
}

/// The start lists the default Provider in the background: a session is
/// created and runs a turn while the listing's answer is held, and takes
/// the reported window at its next turn, its usage measured against it.
/// The next run recovers the session as it was rewritten.
#[tokio::test]
async fn a_session_created_before_the_listing_lands_takes_its_window_at_its_next_turn() {
    let release = Arc::new(tokio::sync::Notify::new());
    let server = FakeServer::with_listings(
        vec![
            (
                with_input_tokens(assistant_events("First."), 1_000),
                Duration::ZERO,
            ),
            (
                with_input_tokens(assistant_events("Second."), 1_000),
                Duration::ZERO,
            ),
        ],
        vec![FakeListing::ok(LISTING).held(release.clone())],
    )
    .await;
    let (paths, config) = configured("model-discovery-late-window", &server.base_url);
    let service = open(&paths, &config, ModelDiscoveryPolicy::every(HOUR, HOUR));
    wait_until("the listing at start", || {
        server.listing_requests().len() == 1
    })
    .await;

    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "first".into())
        .await
        .unwrap();
    let window = |service: &AgentApplicationService| {
        service
            .snapshot(session.id)
            .unwrap()
            .usage
            .unwrap()
            .context_window_tokens
    };
    assert_eq!(window(&service), None, "the listing had not landed");

    release.notify_one();
    wait_until("the listing recorded", || recorded(&service, &server)).await;
    service
        .start_turn(session.id, "second".into())
        .await
        .unwrap();
    assert_eq!(window(&service), Some(123_456));
    assert_eq!(server.listing_requests().len(), 1);
    assert_eq!(server.requests().len(), 2, "the turns' requests only");
    stop(&service);

    // Loading the session checks its usage against its window.
    let reopened = open(&paths, &config, ModelDiscoveryPolicy::OFF);
    reopened.core.activate_session(session.id).unwrap();
    assert_eq!(window(&reopened), Some(123_456));
    stop(&reopened);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A failed listing is not tried again within its backoff, however many
/// sessions ask.
#[tokio::test]
async fn a_failed_listing_is_not_tried_again_within_its_backoff() {
    let server = FakeServer::with_listings(
        Vec::new(),
        vec![FakeListing::failing(404), FakeListing::ok(LISTING)],
    )
    .await;
    let (paths, config) = configured("model-discovery-backoff", &server.base_url);
    let service = open(&paths, &config, ModelDiscoveryPolicy::every(HOUR, HOUR));
    handled(&service, 1).await;
    assert_eq!(server.listing_requests().len(), 1);

    service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    handled(&service, 2).await;

    assert_eq!(server.listing_requests().len(), 1);
    assert!(!recorded(&service, &server));
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// After its backoff a failed listing is tried again; a listing then holds
/// the next one off for its period, in this run and in the next (the
/// recorded listing is kept with the model metadata).
#[tokio::test]
async fn a_listing_holds_the_next_off_for_its_period_across_runs() {
    let backoff = Duration::from_millis(300);
    let server = FakeServer::with_listings(
        Vec::new(),
        vec![
            FakeListing::failing(404),
            FakeListing::ok(LISTING),
            FakeListing::ok(LISTING),
        ],
    )
    .await;
    let (paths, config) = configured("model-discovery-period", &server.base_url);
    let service = open(&paths, &config, ModelDiscoveryPolicy::every(HOUR, backoff));
    handled(&service, 1).await;
    assert_eq!(server.listing_requests().len(), 1);

    tokio::time::sleep(backoff + Duration::from_millis(100)).await;
    service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    handled(&service, 2).await;
    assert_eq!(server.listing_requests().len(), 2);
    assert!(recorded(&service, &server));

    service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    handled(&service, 3).await;
    assert_eq!(server.listing_requests().len(), 2);
    stop(&service);

    let reopened = open(&paths, &config, ModelDiscoveryPolicy::every(HOUR, backoff));
    handled(&reopened, 1).await;
    assert_eq!(server.listing_requests().len(), 2);
    stop(&reopened);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Selecting a model of another Provider lists that Provider.
#[tokio::test]
async fn selecting_another_providers_model_lists_that_provider() {
    let server = FakeServer::with_listings(Vec::new(), vec![FakeListing::ok(LISTING)]).await;
    let other = FakeServer::with_listings(
        Vec::new(),
        vec![FakeListing::ok(r#"{"data":[{"id":"other-model"}]}"#)],
    )
    .await;
    let (paths, config) = configured("model-discovery-selection", &server.base_url);
    config
        .mutate_providers(1, |registry| {
            let mut provider = registry.providers[0].clone();
            provider.id = "provider-2".into();
            provider.display_name = "Other".into();
            provider.base_url = other.base_url.clone();
            registry.providers.push(provider);
            Ok(())
        })
        .unwrap();
    let service = open(&paths, &config, ModelDiscoveryPolicy::every(HOUR, HOUR));
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    handled(&service, 2).await;
    assert_eq!(server.listing_requests().len(), 1);
    assert!(other.listing_requests().is_empty());

    service
        .set_session_model(AgentSessionModelSelectionRequest {
            session_id: session.id,
            generation: session.generation,
            revision: session.revision,
            provider_id: "provider-2".into(),
            model: "other-model".into(),
        })
        .await
        .unwrap();
    handled(&service, 3).await;

    assert_eq!(other.listing_requests().len(), 1);
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// With `modelMetadata.discovery = "off"` nothing is listed by itself, at
/// start, at a session's creation or at a turn; a listing the person asks
/// for still is, and is recorded.
#[tokio::test]
async fn discovery_off_lists_nothing_by_itself() {
    let server = FakeServer::with_listings(
        vec![(assistant_events("Done."), Duration::ZERO)],
        vec![FakeListing::ok(LISTING)],
    )
    .await;
    let (paths, config) = configured("model-discovery-off", &server.base_url);
    config
        .set_user_value("modelMetadata.discovery", "off")
        .unwrap();
    let service = open(&paths, &config, ModelDiscoveryPolicy::every(HOUR, HOUR));
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "hello".into())
        .await
        .unwrap();
    handled(&service, 3).await;
    assert!(server.listing_requests().is_empty());
    assert!(!recorded(&service, &server));

    service
        .list_provider_models(AgentProviderModelCatalogRequest::default())
        .await
        .unwrap();
    assert_eq!(server.listing_requests().len(), 1);
    assert!(recorded(&service, &server));
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Shutdown cancels a listing still waiting for its answer: the worker
/// stops at once, and nothing is recorded.
#[tokio::test]
async fn shutdown_cancels_a_pending_listing() {
    let never = Arc::new(tokio::sync::Notify::new());
    let server =
        FakeServer::with_listings(Vec::new(), vec![FakeListing::ok(LISTING).held(never)]).await;
    let (paths, config) = configured("model-discovery-shutdown", &server.base_url);
    let service = open(&paths, &config, ModelDiscoveryPolicy::every(HOUR, HOUR));
    wait_until("the listing at start", || {
        server.listing_requests().len() == 1
    })
    .await;

    let started = Instant::now();
    stop(&service);

    // The client would wait 30 s for the answer's head.
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(!recorded(&service, &server));
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
