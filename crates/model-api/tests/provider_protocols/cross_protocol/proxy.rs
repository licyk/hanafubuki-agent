//! Every protocol's requests, and the model listing, go through the
//! Provider's proxy: the request reaches the proxy naming the Provider's
//! whole URL, at a host that does not resolve.

use hanafubuki_agent_model_api::{ModelApiProtocol, ModelCatalogClient, ModelHttpProxy};

use super::*;

const PROVIDER: &str = "http://provider.invalid/v1";

/// A Provider at [`PROVIDER`], whose requests go through `proxy`.
fn proxied(proxy: &FakeHttpServer) -> ModelProvider {
    let origin = proxy.base_url().trim_end_matches("/v1").to_string();
    ModelProvider::new(ModelEndpoint::parse(PROVIDER).unwrap(), None)
        .with_proxy(ModelHttpProxy::Explicit(origin))
}

#[tokio::test]
async fn every_protocol_and_the_listing_go_through_the_proxy() {
    let refusal = br#"{"error":{"code":"context_length_exceeded","message":"maximum context length exceeded"}}"#;
    let responses =
        FakeHttpServer::start(vec![PlannedResponse::status(400, refusal.to_vec())]).await;
    let chat = FakeHttpServer::start(vec![PlannedResponse::status(400, refusal.to_vec())]).await;
    let anthropic =
        FakeHttpServer::start(vec![PlannedResponse::status(400, refusal.to_vec())]).await;
    let listing = FakeHttpServer::start(vec![PlannedResponse::status(
        200,
        br#"{"data":[{"id":"proxied-model"}]}"#.to_vec(),
    )])
    .await;

    for transport in [
        Box::new(ResponsesModelTransport::new(proxied(&responses)).unwrap()) as Box<dyn ModelPort>,
        Box::new(ChatCompletionsModelTransport::new(proxied(&chat)).unwrap()),
        Box::new(AnthropicMessagesModelTransport::new(proxied(&anthropic)).unwrap()),
    ] {
        assert!(matches!(
            transport
                .stream(fixture_request(), CancellationToken::default())
                .await,
            Err(ModelError::ContextWindowExceeded { .. })
        ));
    }
    let entries = ModelCatalogClient::new(proxied(&listing))
        .unwrap()
        .list(
            ModelApiProtocol::ChatCompletions,
            CancellationToken::default(),
        )
        .await
        .unwrap()
        .entries;
    assert_eq!(entries[0].id, "proxied-model");

    for (proxy, path) in [
        (&responses, "/responses"),
        (&chat, "/chat/completions"),
        (&anthropic, "/messages"),
        (&listing, "/models"),
    ] {
        let [request] = proxy.requests().try_into().unwrap();
        assert!(
            request.path.starts_with(PROVIDER) && request.path.ends_with(path),
            "{} did not reach the proxy as {PROVIDER}…{path}",
            request.path
        );
    }
}

/// A proxy URL that cannot be used fails the transport's construction
/// without naming the URL, which may hold credentials.
#[test]
fn an_unusable_proxy_is_refused_without_naming_it() {
    let provider = ModelProvider::new(ModelEndpoint::parse(PROVIDER).unwrap(), None)
        .with_proxy(ModelHttpProxy::Explicit("not a proxy:secret".to_string()));
    let error = ChatCompletionsModelTransport::new(provider)
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("secret"), "{error}");
    assert_eq!(
        format!(
            "{:?}",
            ModelHttpProxy::Explicit("http://user:secret@proxy".to_string())
        ),
        "Explicit(<redacted>)"
    );
}
