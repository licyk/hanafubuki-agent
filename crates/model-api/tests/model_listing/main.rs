//! A Provider's model listing through `ModelCatalogClient`: the shapes it
//! reads, its tolerance of entries it cannot use, and its paging.
//!
//! The fixtures are the providers' documented example responses:
//! - `openai.json`: <https://developers.openai.com/api/reference/resources/models/methods/list>
//! - `openrouter.json`: <https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties>
//! - `anthropic.json`: <https://platform.claude.com/docs/en/api/models/list>
//!
//! The first OpenRouter and Anthropic entries are the documented ones; each
//! second entry (`example/…`, `claude-example-…`) is illustrative, in the
//! documented shape with the values the example leaves out (Anthropic's
//! example reports `0` for both limits). The Anthropic page was made the last.

#[path = "../support/mod.rs"]
mod support;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::{
    ModelApiProtocol, ModelCatalogClient, ModelCatalogEntry, ModelCatalogListing, ModelCredential,
    ModelEndpoint, ModelError, ModelHeader, ModelProvider, ModelReasoningEffort,
};
use serde_json::json;
use support::{FakeHttpServer, PlannedResponse};

#[tokio::test]
async fn the_documented_openai_listing_reports_ids_only() {
    let listing = list_one_page(
        ModelApiProtocol::Responses,
        include_str!("fixtures/openai.json"),
    )
    .await
    .unwrap();
    assert_eq!(
        listing,
        ModelCatalogListing {
            entries: ["model-id-0", "model-id-1", "model-id-2"]
                .map(unknown_entry)
                .to_vec(),
            skipped: 0,
        }
    );
}

#[tokio::test]
async fn the_documented_openrouter_listing_reports_limits_and_capabilities() {
    let listing = list_one_page(
        ModelApiProtocol::ChatCompletions,
        include_str!("fixtures/openrouter.json"),
    )
    .await
    .unwrap();
    assert_eq!(listing.skipped, 0);
    let [vision_reasoner, gpt_4] = listing.entries.try_into().unwrap();
    assert_eq!(
        gpt_4,
        ModelCatalogEntry {
            context_window: Some(8192),
            max_output_tokens: Some(4096),
            image_input: Some(false),
            reasoning: Some(false),
            ..unknown_entry("openai/gpt-4")
        }
    );
    assert_eq!(
        vision_reasoner,
        ModelCatalogEntry {
            context_window: Some(400_000),
            max_output_tokens: Some(128_000),
            image_input: Some(true),
            reasoning: Some(true),
            ..unknown_entry("example/vision-reasoner")
        }
    );
}

#[tokio::test]
async fn the_documented_anthropic_listing_reports_limits_and_capabilities() {
    let listing = list_one_page(
        ModelApiProtocol::AnthropicMessages,
        include_str!("fixtures/anthropic.json"),
    )
    .await
    .unwrap();
    assert_eq!(listing.skipped, 0);
    let [example, opus] = listing.entries.try_into().unwrap();
    assert_eq!(
        opus,
        ModelCatalogEntry {
            display_name: Some("Claude Opus 5.5".into()),
            image_input: Some(true),
            reasoning: Some(true),
            ..unknown_entry("claude-opus-5-5")
        }
    );
    assert_eq!(
        example,
        ModelCatalogEntry {
            display_name: Some("Claude Example (text)".into()),
            context_window: Some(200_000),
            max_output_tokens: Some(64_000),
            image_input: Some(false),
            reasoning: Some(false),
            ..unknown_entry("claude-example-text")
        }
    );
}

#[tokio::test]
async fn entries_without_a_usable_id_are_skipped_and_counted() {
    let page = json!({"data": [
        {"id": "model-b", "context_length": "large", "vision": "yes"},
        {"display_name": "No id"},
        {"id": 7},
        {"id": " "},
        {"id": "m".repeat(513)},
        "model-c",
        {"id": "model-a", "default_reasoning_level": "none", "supported_reasoning_levels": [{"effort": "low"}]},
        {"id": "model-b", "context_length": 4096},
    ]});
    let listing = list_one_page(ModelApiProtocol::ChatCompletions, &page.to_string())
        .await
        .unwrap();
    assert_eq!(listing.skipped, 5);
    assert_eq!(
        listing.entries,
        [
            ModelCatalogEntry {
                supported_reasoning_efforts: vec![ModelReasoningEffort::Low],
                reasoning: Some(true),
                ..unknown_entry("model-a")
            },
            unknown_entry("model-b"),
        ]
    );
}

#[tokio::test]
async fn a_page_without_a_data_array_fails_the_listing() {
    for body in [r#"{"object":"list"}"#, r#"{"data":{"id":"model-a"}}"#, "[]"] {
        assert!(
            matches!(
                list_one_page(ModelApiProtocol::ChatCompletions, body).await,
                Err(ModelError::Protocol(message)) if message.contains("no data array")
            ),
            "{body}"
        );
    }
}

#[tokio::test]
async fn model_catalog_supports_openai_and_paginated_anthropic_shapes() {
    let openai = FakeHttpServer::start(vec![PlannedResponse::status(
        200,
        br#"{"object":"list","data":[{"id":"z-model","default_reasoning_level":"medium","supported_reasoning_levels":[{"effort":"low","description":"Fast"},{"effort":"future-tier","description":"Future"}],"context_length":131072},{"id":"a-model","context_length":"large"}]}"#.to_vec(),
    )])
    .await;
    let entries = ModelCatalogClient::new(provider(&openai))
        .unwrap()
        .list(
            ModelApiProtocol::ChatCompletions,
            CancellationToken::default(),
        )
        .await
        .unwrap()
        .entries;
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a-model", "z-model"]
    );
    assert_eq!(openai.requests()[0].path, "/v1/models");
    assert_eq!(entries[0].context_window, None);
    assert_eq!(entries[1].context_window, Some(131_072));
    let serialized = serde_json::to_value(&entries[1]).unwrap();
    assert_eq!(serialized["defaultReasoningEffort"], "medium");
    assert_eq!(
        serialized["supportedReasoningEfforts"],
        json!(["low", "future-tier"])
    );
    assert_eq!(serialized["reasoning"], true);
    assert_eq!(
        openai.requests()[0].headers.get("authorization").unwrap(),
        "Bearer provider-secret"
    );
    assert_eq!(
        openai.requests()[0]
            .headers
            .get("x-provider-scope")
            .unwrap(),
        "workspace-a"
    );

    let anthropic = FakeHttpServer::start(vec![
        PlannedResponse::status(
            200,
            br#"{"data":[{"id":"claude-b","display_name":"Claude B","max_input_tokens":1000000}],"has_more":true,"last_id":"claude-b"}"#.to_vec(),
        ),
        PlannedResponse::status(
            200,
            br#"{"data":[{"id":"claude-a","display_name":"Claude A"}],"has_more":false,"last_id":"claude-a"}"#.to_vec(),
        ),
    ])
    .await;
    let entries = ModelCatalogClient::new(provider(&anthropic))
        .unwrap()
        .list(
            ModelApiProtocol::AnthropicMessages,
            CancellationToken::default(),
        )
        .await
        .unwrap()
        .entries;
    assert_eq!(entries[0].display_name.as_deref(), Some("Claude A"));
    assert_eq!(entries[0].context_window, None);
    assert_eq!(entries[1].context_window, Some(1_000_000));
    let requests = anthropic.requests();
    assert_eq!(requests[0].path, "/v1/models?limit=100");
    assert_eq!(requests[1].path, "/v1/models?limit=100&after_id=claude-b");
    assert_eq!(
        requests[0].headers.get("x-api-key").unwrap(),
        "provider-secret"
    );
    assert_eq!(
        requests[0].headers.get("x-provider-scope").unwrap(),
        "workspace-a"
    );
}

#[tokio::test]
async fn an_anthropic_page_with_more_but_no_last_id_ends_the_listing() {
    let server = FakeHttpServer::start(vec![PlannedResponse::status(
        200,
        br#"{"data":[{"id":"claude-a"}],"has_more":true,"last_id":null}"#.to_vec(),
    )])
    .await;
    let listing = ModelCatalogClient::new(provider(&server))
        .unwrap()
        .list(
            ModelApiProtocol::AnthropicMessages,
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(listing.entries, [unknown_entry("claude-a")]);
    assert_eq!(server.requests().len(), 1);
}

async fn list_one_page(
    protocol: ModelApiProtocol,
    body: &str,
) -> Result<ModelCatalogListing, ModelError> {
    let server = FakeHttpServer::start(vec![PlannedResponse::status(200, body.as_bytes())]).await;
    ModelCatalogClient::new(provider(&server))
        .unwrap()
        .list(protocol, CancellationToken::default())
        .await
}

fn unknown_entry(id: &str) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: id.into(),
        display_name: None,
        default_reasoning_effort: None,
        supported_reasoning_efforts: Vec::new(),
        context_window: None,
        max_output_tokens: None,
        image_input: None,
        reasoning: None,
    }
}

fn provider(server: &FakeHttpServer) -> ModelProvider {
    ModelProvider::new(
        ModelEndpoint::parse(server.base_url()).unwrap(),
        Some(ModelCredential::new("provider-secret").unwrap()),
    )
    .with_header(ModelHeader::new("x-provider-scope", "workspace-a").unwrap())
}
