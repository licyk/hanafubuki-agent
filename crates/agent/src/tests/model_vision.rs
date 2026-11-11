//! Image input from what the model metadata knows, against a fake Provider
//! and through the real `view_image` plugin: a model its listing says reads
//! no images is offered no `view_image` and sent no image, with no request
//! refused first, and a turn started with one is refused; a model that
//! refused an image in one run is sent none in the next
//! (`model-metadata.json`), unless that refusal is forgotten; and what the
//! person configures for the model (`models."<id>"`) beats the listing,
//! from a live session's next turn.

use std::io::Cursor;

use hanafubuki_agent_config::{AgentModelImageInput, AgentModelOverrideSettings};
use hanafubuki_agent_kernel::{
    AgentError, AgentImageAttachmentRef, AgentImageDetail, AgentSessionId, AgentSessionMode,
    ModelMetadataSource, PreparedAgentImage,
};
use hanafubuki_agent_process::executor::ExecutorManager;
use image::{DynamicImage, ImageFormat};

use super::*;
use crate::test_support::TestPaths;

const TEXT_ONLY: &str = r#"{"data":[{"id":"fixture-model","input_modalities":["text"]}]}"#;
/// The Provider's refusal of the request's image, in its stream before
/// anything else.
const IMAGE_REFUSAL: &str = concat!(
    "data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp-refused\",",
    "\"error\":{\"code\":\"unsupported_image\",\"message\":\"image input is not supported\"}}}\n\n"
);
const REFUSED: &str = "the configured model does not support image input";

fn png() -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    DynamicImage::new_rgba8(1, 1)
        .write_to(&mut output, ImageFormat::Png)
        .unwrap();
    output.into_inner()
}

fn staged(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) -> AgentImageAttachmentRef {
    service
        .core
        .stage_image_attachment(
            session_id,
            PreparedAgentImage {
                bytes: png(),
                mime_type: "image/png".to_string(),
                width: 1,
                height: 1,
                display_name: "fixture.png".to_string(),
                detail: AgentImageDetail::High,
            },
        )
        .unwrap()
}

fn with_image(text: &str, image: AgentImageAttachmentRef) -> AgentTurnInput {
    AgentTurnInput {
        images: vec![image],
        ..AgentTurnInput::from(text)
    }
}

fn body(request: &CapturedRequest) -> serde_json::Value {
    serde_json::from_slice(&request.body).unwrap()
}

fn offers_view_image(request: &CapturedRequest) -> bool {
    body(request)["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "view_image")
}

fn sends_an_image(request: &CapturedRequest) -> bool {
    body(request).to_string().contains("\"input_image\"")
}

fn takes_images(service: &AgentApplicationService, session_id: AgentSessionId) -> bool {
    service
        .snapshot(session_id)
        .unwrap()
        .input_capabilities
        .image_input
}

/// A new Agent over `paths`' storage, as after a restart.
fn reopen(config: AgentConfig, paths: &TestPaths) -> AgentApplicationService {
    AgentApplicationService::new_with_host(
        config,
        test_host(paths),
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
    )
    .unwrap()
}

fn stop(service: &AgentApplicationService) {
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
}

/// A session sends an image while nothing is known of its model; then the
/// Provider's listing says the model reads text only. The next turn's one
/// request offers no `view_image` and carries a placeholder in the image's
/// place, and a turn started with an image is refused before any request.
#[tokio::test]
async fn a_model_listed_without_image_input_is_offered_no_view_image_and_sent_no_image() {
    let server = FakeServer::with_listings(
        vec![
            (assistant_events("Seen."), Duration::ZERO),
            (assistant_events("Done."), Duration::ZERO),
        ],
        vec![FakeListing::ok(TEXT_ONLY)],
    )
    .await;
    let (service, paths) = configured_service("model-vision-listed", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let image = staged(&service, session.id);
    service
        .start_turn(session.id, with_image("look", image))
        .await
        .unwrap();
    let first = &server.requests()[0];
    assert!(offers_view_image(first) && sends_an_image(first));

    service
        .list_provider_models(AgentProviderModelCatalogRequest::default())
        .await
        .unwrap();

    assert!(!takes_images(&service, session.id));
    let image = staged(&service, session.id);
    assert_eq!(
        service
            .start_turn(session.id, with_image("look again", image))
            .await
            .unwrap_err(),
        AgentError::InvalidInput(REFUSED.to_string())
    );
    service
        .start_turn(session.id, "and now?".into())
        .await
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 2, "no request was refused");
    assert!(!offers_view_image(&requests[1]));
    assert!(!sends_an_image(&requests[1]));
    assert!(String::from_utf8_lossy(&requests[1].body).contains("image content omitted"));
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The Provider refuses an image: the turn goes on without it, and the
/// next run, over the same data directory, sends that model no image and
/// offers it no `view_image` from a new session's first request.
#[tokio::test]
async fn an_image_refused_in_one_run_is_not_sent_in_the_next() {
    let server = FakeServer::start(vec![
        (IMAGE_REFUSAL.as_bytes().to_vec(), Duration::ZERO),
        (assistant_events("Seen as text."), Duration::ZERO),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let (service, paths, config) = service_with_config("model-vision-restart", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let image = staged(&service, session.id);
    let outcome = service
        .start_turn(session.id, with_image("look", image))
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Seen as text.");
    let requests = server.requests();
    assert!(sends_an_image(&requests[0]) && !sends_an_image(&requests[1]));
    stop(&service);

    let reopened = reopen(config, &paths);
    let session = reopened
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert!(!takes_images(&reopened, session.id));
    reopened
        .start_turn(session.id, "hello".into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert!(!offers_view_image(&requests[2]));
    stop(&reopened);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The Provider refuses an image in one run, and the person forgets what
/// it taught: the next run sends the model images again.
#[tokio::test]
async fn a_forgotten_image_refusal_is_not_kept_by_the_next_run() {
    let server = FakeServer::start(vec![
        (IMAGE_REFUSAL.as_bytes().to_vec(), Duration::ZERO),
        (assistant_events("Seen as text."), Duration::ZERO),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let (service, paths, config) = service_with_config("model-vision-forget", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let image = staged(&service, session.id);
    service
        .start_turn(session.id, with_image("look", image))
        .await
        .unwrap();
    assert!(!takes_images(&service, session.id));

    assert_eq!(
        service
            .forget_learned_model_facts("provider-1", None)
            .await
            .unwrap(),
        1
    );
    stop(&service);

    let reopened = reopen(config, &paths);
    let session = reopened
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    assert!(takes_images(&reopened, session.id));
    reopened
        .start_turn(session.id, "hello".into())
        .await
        .unwrap();
    assert!(offers_view_image(&server.requests()[2]));
    stop(&reopened);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The listing says the model reads text only; the person configures it as
/// reading images, with a window, in `providers.toml` while a session is
/// open: its next turn offers `view_image` and sends the image, and takes
/// the window as configured. An edit of the Provider keeps the setting;
/// once it is removed, the next turn sends no image and the window is
/// unknown again.
#[tokio::test]
async fn a_per_model_override_reaches_a_live_session_at_its_next_turn() {
    let server = FakeServer::with_listings(
        vec![
            (assistant_events("Seen."), Duration::ZERO),
            (assistant_events("Done."), Duration::ZERO),
        ],
        vec![FakeListing::ok(TEXT_ONLY)],
    )
    .await;
    let (service, paths, config) = service_with_config("model-vision-override", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .list_provider_models(AgentProviderModelCatalogRequest::default())
        .await
        .unwrap();
    assert!(!takes_images(&service, session.id));
    let set = |models: Option<AgentModelOverrideSettings>| {
        let revision = config.providers().unwrap().revision;
        config
            .mutate_providers(revision, |registry| {
                registry.providers[0].models = models
                    .into_iter()
                    .map(|models| ("fixture-model".to_string(), models))
                    .collect();
                Ok(())
            })
            .unwrap();
    };
    set(Some(AgentModelOverrideSettings {
        context_window_tokens: Some(50_000),
        image_input: AgentModelImageInput::Supported,
        ..AgentModelOverrideSettings::default()
    }));

    let image = staged(&service, session.id);
    service
        .start_turn(session.id, with_image("look", image))
        .await
        .unwrap();
    let first = &server.requests()[0];
    assert!(offers_view_image(first) && sends_an_image(first));
    let metadata = service.session_model_metadata(session.id).unwrap();
    let configured = ModelMetadataSource::Configured;
    assert_eq!(
        metadata
            .context_window
            .map(|fact| (fact.value, fact.source)),
        Some((50_000, configured))
    );
    assert_eq!(
        metadata.image_input.map(|fact| (fact.value, fact.source)),
        Some((true, configured))
    );
    let registry = service.provider_registry_snapshot().await.unwrap();
    let edited = service
        .write_provider(AgentProviderWriteRequest {
            expected_revision: registry.revision,
            provider_id: Some("provider-1".to_string()),
            provider: AgentProviderDraft {
                display_name: "Renamed".to_string(),
                enabled: true,
                protocol: AgentModelProtocol::Responses,
                base_url: server.base_url.clone(),
                api_token: None,
                request_headers: Vec::new(),
            },
        })
        .await
        .unwrap();
    assert!(edited.providers[0].models.contains_key("fixture-model"));

    set(None);
    service
        .start_turn(session.id, "and now?".into())
        .await
        .unwrap();
    assert!(!offers_view_image(&server.requests()[1]));
    assert!(!takes_images(&service, session.id));
    let metadata = service.session_model_metadata(session.id).unwrap();
    assert_eq!(metadata.context_window, None);
    stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
