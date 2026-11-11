//! Model discovery against a fake Provider that answers `GET /models` apart
//! from the turns' event streams: the Agent lists the default Provider at
//! start, in the background, a session takes what the listing reported at
//! its next turn, and the listing is rate-limited, turned off by
//! `modelMetadata.discovery = "off"` and cancelled at shutdown. The same
//! worker fetches models.dev's catalogue when `modelMetadata.modelsDev` is
//! on, from a fake server too.

use hanafubuki_agent_kernel::{
    AgentSessionId, AgentSessionMode, ModelApiProtocol, ModelMetadataSource,
};
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

/// A catalogue in models.dev's form that knows the test model: under
/// another provider, and under one keyed `own` with a larger window.
const MODELS_DEV: &str = r#"{"elsewhere": {"api": "https://api.elsewhere.test/v1", "models": {
    "fixture-model": {"reasoning": false, "modalities": {"input": ["text", "image"]},
                      "limit": {"context": 333333, "output": 4096}}}},
  "own": {"models": {"fixture-model": {"limit": {"context": 444444}}}}}"#;

/// The URL a test's models.dev answers at: a fake server's `…/models`,
/// which it answers apart from the turns' event streams.
fn models_dev_url(server: &FakeServer) -> String {
    format!("{}/models", server.base_url)
}

/// The window the session `session` has, and its source.
fn session_window(
    service: &AgentApplicationService,
    session: AgentSessionId,
) -> Option<(u64, ModelMetadataSource)> {
    service
        .session_model_metadata(session)
        .unwrap()
        .context_window
        .map(|fact| (fact.value, fact.source))
}

/// models.dev is off by default: however many requests the worker
/// handles, it is never asked, and nothing is said of it.
#[tokio::test]
async fn models_dev_is_never_asked_by_default() {
    let models_dev = FakeServer::with_listings(Vec::new(), vec![FakeListing::ok(MODELS_DEV)]).await;
    let server = FakeServer::start(vec![(assistant_events("Done."), Duration::ZERO)]).await;
    let paths = test_paths("models-dev-off");
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, &server.base_url, None);
    let policy =
        ModelDiscoveryPolicy::every(HOUR, HOUR).with_models_dev(models_dev_url(&models_dev));
    let service = open(&paths, &config, policy);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "hello".into())
        .await
        .unwrap();
    handled(&service, 3).await;

    assert!(models_dev.listing_requests().is_empty());
    assert!(service.core.models_dev_fetched_at().is_none());
    assert_eq!(session_window(&service, session.id), None);
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Turned on (discovery off: the two are apart), models.dev is fetched in
/// the background with the Agent's User-Agent, a session takes the window
/// it says at its next turn, and the copy kept in `model-metadata.json`
/// spares the next run a download within the day.
#[tokio::test]
async fn models_dev_is_fetched_once_a_day_and_fills_what_nothing_else_says() {
    // A second answer, so a second download would be answered and seen.
    let models_dev = FakeServer::with_listings(
        Vec::new(),
        vec![FakeListing::ok(MODELS_DEV), FakeListing::ok(MODELS_DEV)],
    )
    .await;
    let server = FakeServer::start(vec![
        (assistant_events("First."), Duration::ZERO),
        (assistant_events("Second."), Duration::ZERO),
    ])
    .await;
    let paths = test_paths("models-dev-on");
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, &server.base_url, None);
    config
        .set_user_value("modelMetadata.modelsDev", "true")
        .unwrap();
    let policy =
        ModelDiscoveryPolicy::every(HOUR, HOUR).with_models_dev(models_dev_url(&models_dev));
    let service = open(&paths, &config, policy.clone());
    wait_until("models.dev fetched", || {
        service.core.models_dev_fetched_at().is_some()
    })
    .await;
    let [request] = models_dev.listing_requests().try_into().ok().unwrap();
    assert_eq!(
        request.headers["user-agent"],
        format!("hanafubuki-agent/{}", env!("CARGO_PKG_VERSION"))
    );
    assert!(!request.headers.contains_key("authorization"));

    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "first".into())
        .await
        .unwrap();
    assert_eq!(
        session_window(&service, session.id),
        Some((333_333, ModelMetadataSource::Catalog))
    );
    let facts = service
        .model_metadata("provider-1", "fixture-model")
        .await
        .unwrap();
    assert_eq!(
        facts.image_input.map(|fact| (fact.value, fact.source)),
        Some((true, ModelMetadataSource::Catalog))
    );
    // Forgetting what refusals taught keeps models.dev's copy.
    service
        .forget_learned_model_facts("provider-1", None)
        .await
        .unwrap();
    let fetched_at = service.core.models_dev_fetched_at();
    assert!(fetched_at.is_some());
    stop(&service);

    let reopened = open(&paths, &config, policy);
    handled(&reopened, 1).await;
    assert_eq!(
        models_dev.listing_requests().len(),
        1,
        "the copy is a day old at most"
    );
    assert_eq!(reopened.core.models_dev_fetched_at(), fetched_at);

    // Renamed `own`, the Provider has its own entry, which a session bound
    // to it and the facts read for it both take over the other's.
    let revision = config.providers().unwrap().revision;
    config
        .mutate_providers(revision, |registry| {
            registry.providers[0].id = "own".into();
            registry.default_provider_id = Some("own".into());
            Ok(())
        })
        .unwrap();
    let own = reopened
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert_eq!(
        session_window(&reopened, own.id),
        Some((444_444, ModelMetadataSource::Catalog))
    );
    let facts = reopened
        .model_metadata("own", "fixture-model")
        .await
        .unwrap();
    assert_eq!(facts.context_window.map(|fact| fact.value), Some(444_444));
    stop(&reopened);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// models.dev unreachable is silent: no turn fails, and the copy kept goes
/// on answering. Turned off, the copy is dropped, and with it the window a
/// session took from it.
#[tokio::test]
async fn models_dev_unreachable_keeps_the_copy_and_off_drops_it() {
    let models_dev = FakeServer::with_listings(Vec::new(), vec![FakeListing::ok(MODELS_DEV)]).await;
    let server = FakeServer::start(vec![
        (assistant_events("First."), Duration::ZERO),
        (assistant_events("Second."), Duration::ZERO),
    ])
    .await;
    let paths = test_paths("models-dev-unreachable");
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, &server.base_url, None);
    config
        .set_user_value("modelMetadata.modelsDev", "true")
        .unwrap();
    let service = open(
        &paths,
        &config,
        ModelDiscoveryPolicy::every(HOUR, HOUR).with_models_dev(models_dev_url(&models_dev)),
    );
    wait_until("models.dev fetched", || {
        service.core.models_dev_fetched_at().is_some()
    })
    .await;
    let fetched_at = service.core.models_dev_fetched_at();
    stop(&service);

    // Due again at once, and nothing answers.
    let reopened = open(
        &paths,
        &config,
        ModelDiscoveryPolicy::every(Duration::ZERO, HOUR),
    );
    let session = reopened
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    reopened
        .start_turn(session.id, "hello".into())
        .await
        .unwrap();
    handled(&reopened, 3).await;
    assert_eq!(reopened.core.models_dev_fetched_at(), fetched_at);
    assert_eq!(
        session_window(&reopened, session.id),
        Some((333_333, ModelMetadataSource::Catalog))
    );

    // Off, the copy is dropped, and the session no longer has its window
    // from the next turn.
    config
        .set_user_value("modelMetadata.modelsDev", "false")
        .unwrap();
    reopened
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    handled(&reopened, 4).await;
    assert!(reopened.core.models_dev_fetched_at().is_none());
    reopened
        .start_turn(session.id, "again".into())
        .await
        .unwrap();
    assert_eq!(session_window(&reopened, session.id), None);
    stop(&reopened);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
