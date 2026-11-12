//! models.dev's catalogue: decoded tolerantly from its own form, looked up
//! by normalized id under the Provider's own entry first, and fetched with
//! the Agent's User-Agent from a local server, directly or through a proxy.
//!
//! `fixtures/api.json` is in the shape `https://models.dev/api.json`
//! publishes (a provider object with `api`, `models`, and per model
//! `reasoning`, `modalities` and `limit`), trimmed to a few providers, with
//! entries that do not read added.

#[path = "../support/mod.rs"]
mod support;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::{
    fetch_models_dev, ModelEndpoint, ModelError, ModelHttpProxy, ModelMetadata,
    ModelMetadataSource, ModelProviderNames, ModelReasoningSupport, ModelsDevCatalog, Sourced,
};
use support::{FakeHttpServer, PlannedResponse};

const FIXTURE: &str = include_str!("fixtures/api.json");
const AT: u64 = 1_000;

fn catalog() -> ModelsDevCatalog {
    ModelsDevCatalog::from_api_json(FIXTURE.as_bytes()).unwrap()
}

fn names(id: &str, base_url: &str) -> ModelProviderNames {
    ModelProviderNames::new(id, &ModelEndpoint::parse(base_url).unwrap())
}

fn models_dev<T>(value: T) -> Option<Sourced<T>> {
    Some(Sourced {
        value,
        source: ModelMetadataSource::Catalog,
        observed_at_ms: AT,
    })
}

fn reasons() -> Option<Sourced<ModelReasoningSupport>> {
    models_dev(ModelReasoningSupport::Supported {
        efforts: Vec::new(),
        default: None,
    })
}

#[test]
fn the_catalogue_reads_each_provider_and_model_on_its_own() {
    let catalog = catalog();
    // Every model of the fixture but the one that is not an object; the
    // two OpenRouter entries keep their own normalized ids.
    assert_eq!(catalog.model_count(), 8);
    assert_eq!(
        catalog.metadata(None, "half-known", AT),
        ModelMetadata {
            max_output_tokens: models_dev(4096),
            ..ModelMetadata::default()
        },
        "a fact that does not read is unknown, the others kept"
    );
    assert_eq!(
        catalog.metadata(None, "nowhere", AT),
        ModelMetadata::default()
    );
}

#[test]
fn a_document_without_providers_or_models_is_refused() {
    for body in [
        "",
        "[]",
        "{}",
        r#"{"empty": {"models": {}}}"#,
        r#"{"x": 1}"#,
    ] {
        assert!(
            matches!(
                ModelsDevCatalog::from_api_json(body.as_bytes()),
                Err(ModelError::Protocol(_))
            ),
            "{body}"
        );
    }
}

#[test]
fn the_providers_own_entry_comes_first() {
    let catalog = catalog();
    let openai = names("openai", "https://api.openai.com/v1");
    assert_eq!(
        catalog.metadata(Some(&openai), "gpt-4o", AT),
        ModelMetadata {
            context_window: models_dev(128_000),
            max_output_tokens: models_dev(16_384),
            reasoning: None,
            image_input: models_dev(true),
        },
        "a no to reasoning is not kept"
    );
    // By the host its API is at, whatever the Provider is called.
    let copilot = names("work", "https://api.githubcopilot.com");
    assert_eq!(
        catalog
            .metadata(Some(&copilot), "gpt-4o", AT)
            .context_window,
        models_dev(64_000)
    );
    // By a label of the base URL's host.
    let deepseek = names("ds", "https://api.deepseek.com/v1");
    assert_eq!(
        catalog
            .metadata(Some(&deepseek), "deepseek-reasoner", AT)
            .max_output_tokens,
        models_dev(64_000)
    );
    let openrouter = names("openrouter", "https://openrouter.ai/api/v1");
    assert_eq!(
        catalog
            .metadata(Some(&openrouter), "deepseek/deepseek-reasoner", AT)
            .context_window,
        models_dev(163_840)
    );
}

#[test]
fn without_its_own_entry_every_providers_is_taken_cautiously() {
    let catalog = catalog();
    let unknown = names("proxy", "http://127.0.0.1:4000/v1");
    for provider in [None, Some(&unknown)] {
        let gpt = catalog.metadata(provider, "GPT-4o", AT);
        assert_eq!(gpt.context_window, models_dev(64_000), "the smallest");
        assert_eq!(gpt.image_input, models_dev(true));
        let reasoner = catalog.metadata(provider, "deepseek-reasoner", AT);
        assert_eq!(reasoner.context_window, models_dev(128_000));
        assert_eq!(reasoner.reasoning, reasons());
    }
    // A vendor prefix, a date and `-latest` are not compared.
    let haiku = catalog.metadata(Some(&unknown), "anthropic/claude-3-5-haiku-20241022", AT);
    assert_eq!(haiku.context_window, models_dev(200_000));
    assert_eq!(
        catalog
            .metadata(None, "claude-3-5-haiku-latest", AT)
            .max_output_tokens,
        models_dev(8_192)
    );
}

#[test]
fn the_kept_catalogue_reads_back_as_it_was() {
    let catalog = catalog();
    let kept = serde_json::to_value(&catalog).unwrap();
    assert_eq!(
        serde_json::from_value::<ModelsDevCatalog>(kept.clone()).unwrap(),
        catalog
    );
    // One provider or model that does not read is left out, and only it.
    let mut damaged = kept;
    damaged["providers"]["openai"]["models"]["gpt-4o"] = serde_json::json!("damaged");
    damaged["providers"]["anthropic"] = serde_json::json!(7);
    let read: ModelsDevCatalog = serde_json::from_value(damaged).unwrap();
    assert_eq!(read.model_count(), catalog.model_count() - 2);
    assert_eq!(read.metadata(None, "o3", AT).reasoning, reasons());
}

#[tokio::test]
async fn the_catalogue_is_fetched_with_the_agents_user_agent_and_no_credential() {
    let server = FakeHttpServer::start(vec![PlannedResponse::status(200, FIXTURE)]).await;
    let url = format!("{}/api.json", server.base_url());

    let fetched = fetch_models_dev(
        &url,
        &ModelHttpProxy::Disabled,
        &CancellationToken::default(),
    )
    .await
    .unwrap();

    assert_eq!(fetched, catalog());
    let [request] = server.requests().try_into().unwrap();
    assert_eq!(
        (request.method.as_str(), request.path.as_str()),
        ("GET", "/v1/api.json")
    );
    assert_eq!(
        request.headers["user-agent"],
        format!("hanafubuki-agent/{}", env!("CARGO_PKG_VERSION"))
    );
    assert!(!request.headers.contains_key("authorization"));
}

/// The download goes through the proxy it is given: the request reaches
/// the proxy, naming models.dev's whole URL, though that host does not
/// resolve; a proxy URL that is not usable fails the download.
#[tokio::test]
async fn the_catalogue_is_fetched_through_the_proxy() {
    let proxy = FakeHttpServer::start(vec![PlannedResponse::status(200, FIXTURE)]).await;
    let address = proxy.base_url().trim_end_matches("/v1").to_string();
    let url = "http://models-dev.invalid/api.json";

    let fetched = fetch_models_dev(
        url,
        &ModelHttpProxy::Explicit(address),
        &CancellationToken::default(),
    )
    .await
    .unwrap();

    assert_eq!(fetched, catalog());
    let [request] = proxy.requests().try_into().unwrap();
    assert_eq!(request.path, url);
    assert!(matches!(
        fetch_models_dev(
            url,
            &ModelHttpProxy::Explicit("not a proxy".to_string()),
            &CancellationToken::default(),
        )
        .await,
        Err(ModelError::InvalidRequest(_))
    ));
}

#[tokio::test]
async fn a_refusal_bad_data_or_no_server_is_an_error() {
    let server = FakeHttpServer::start(vec![
        PlannedResponse::status(503, "{}"),
        PlannedResponse::status(200, "<html>not json</html>"),
    ])
    .await;
    let url = format!("{}/api.json", server.base_url());
    for _ in 0..2 {
        assert!(fetch_models_dev(
            &url,
            &ModelHttpProxy::Disabled,
            &CancellationToken::default()
        )
        .await
        .is_err());
    }
    // Nothing listens on the discard port.
    assert!(fetch_models_dev(
        "http://127.0.0.1:9/api.json",
        &ModelHttpProxy::Disabled,
        &CancellationToken::default(),
    )
    .await
    .is_err());
}

#[tokio::test]
async fn a_fetch_is_cancelled() {
    let server = FakeHttpServer::start(vec![PlannedResponse {
        header_delay: std::time::Duration::from_secs(30),
        ..PlannedResponse::status(200, FIXTURE)
    }])
    .await;
    let url = format!("{}/api.json", server.base_url());
    let cancellation = CancellationToken::default();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();
    });
    let started = std::time::Instant::now();
    assert!(matches!(
        fetch_models_dev(&url, &ModelHttpProxy::Disabled, &cancellation).await,
        Err(ModelError::Cancelled)
    ));
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
