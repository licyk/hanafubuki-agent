//! A Provider's model listing through `ModelCatalogClient`: the shapes it
//! reads, its tolerance of entries it cannot use, and its paging.
//!
//! The fixtures are the providers' documented example responses:
//! - `openai.json`: <https://developers.openai.com/api/reference/resources/models/methods/list>
//! - `openrouter.json`: <https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties>
//! - `anthropic.json`: <https://platform.claude.com/docs/en/api/models/list>
//! - `mistral.json`: <https://docs.mistral.ai/api/endpoint/models>, the
//!   example of `GET /v1/models` (`openapi.yaml`, `userExample`), which
//!   shows the cards bare; here in the `ModelList` envelope
//!   (`{"object": "list", "data": [...]}`) the same schema names.
//! - `zai.json`: <https://open.bigmodel.cn/api/v1/models>' own answer, read
//!   on 2026-11-20: z.ai publishes no example, and the page is the Codex
//!   envelope (`{"models": [...]}`) whose entries name their model in
//!   `slug`.
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
async fn a_large_catalog_keeps_every_model_without_entry_or_body_limits() {
    let data = (0..2_100)
        .map(|index| {
            json!({
                "id": format!("model-{index:04}"),
                "description": "x".repeat(1024),
                "context_length": 1_000_000,
            })
        })
        .collect::<Vec<_>>();
    let body = json!({"data": data}).to_string();
    assert!(body.len() > 2 * 1024 * 1024);
    for protocol in [
        ModelApiProtocol::Responses,
        ModelApiProtocol::ChatCompletions,
    ] {
        let listing = list_one_page(protocol, &body).await.unwrap();
        assert_eq!(listing.entries.len(), 2_100);
        assert_eq!(listing.skipped, 0);
        for (index, entry) in listing.entries.iter().enumerate() {
            assert_eq!(entry.id, format!("model-{index:04}"));
            assert_eq!(entry.context_window, Some(1_000_000));
        }
    }
}

#[tokio::test]
async fn anthropic_catalogs_continue_past_ten_pages() {
    let responses = (0..12)
        .map(|page| {
            let data = (0..100)
                .map(|index| json!({"id": format!("model-{:04}", page * 100 + index)}))
                .collect::<Vec<_>>();
            PlannedResponse::status(
                200,
                json!({
                    "data": data, "has_more": page < 11,
                    "last_id": format!("model-{:04}", page * 100 + 99),
                })
                .to_string()
                .into_bytes(),
            )
        })
        .collect();
    let server = FakeHttpServer::start(responses).await;
    let listing = ModelCatalogClient::new(provider(&server))
        .unwrap()
        .list(
            ModelApiProtocol::AnthropicMessages,
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(listing.entries.len(), 1_200);
    let requests = server.requests();
    assert_eq!(requests.len(), 12);
    assert_eq!(
        requests[11].path,
        "/v1/models?limit=100&after_id=model-1099"
    );
    assert_eq!(listing.entries.last().unwrap().id, "model-1199");
}

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
            // `reasoning.supported_efforts`, highest first, on the ladder.
            supported_reasoning_efforts: efforts(&["minimal", "low", "medium", "high"]),
            default_reasoning_effort: Some(ModelReasoningEffort::Medium),
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
            // `capabilities.effort.<level>.supported`, on the ladder.
            supported_reasoning_efforts: efforts(&["low", "medium", "high", "xhigh", "max"]),
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
async fn the_documented_mistral_listing_reports_its_window() {
    let listing = list_one_page(
        ModelApiProtocol::ChatCompletions,
        include_str!("fixtures/mistral.json"),
    )
    .await
    .unwrap();
    assert_eq!(listing.skipped, 0);
    assert_eq!(
        listing.entries,
        [ModelCatalogEntry {
            // `max_context_length`; `capabilities.vision`.
            context_window: Some(32_768),
            image_input: Some(false),
            ..unknown_entry("<model_id>")
        }]
    );
}

#[tokio::test]
async fn the_zai_listing_names_its_models_by_slug() {
    let listing = list_one_page(
        ModelApiProtocol::Responses,
        include_str!("fixtures/zai.json"),
    )
    .await
    .unwrap();
    assert_eq!(listing.skipped, 0);
    // Sorted by display name: `glm-5-turbo` before `glm-5.3`.
    let [turbo, flagship, flash] = listing.entries.try_into().unwrap();
    assert_eq!(
        turbo,
        ModelCatalogEntry {
            display_name: Some("glm-5-turbo".into()),
            // It reports a default level and no levels it supports.
            default_reasoning_effort: Some(ModelReasoningEffort::parse("max").unwrap()),
            context_window: Some(204_800),
            image_input: Some(false),
            reasoning: Some(true),
            ..unknown_entry("glm-5-turbo")
        }
    );
    assert_eq!(
        flagship,
        ModelCatalogEntry {
            display_name: Some("glm-5.3".into()),
            default_reasoning_effort: Some(ModelReasoningEffort::parse("max").unwrap()),
            supported_reasoning_efforts: efforts(&["low", "high", "max"]),
            context_window: Some(1_048_576),
            image_input: Some(false),
            reasoning: Some(true),
            ..unknown_entry("glm-5.3")
        }
    );
    assert_eq!(
        flash,
        ModelCatalogEntry {
            display_name: Some("glm-5.3-flash".into()),
            default_reasoning_effort: Some(ModelReasoningEffort::parse("max").unwrap()),
            supported_reasoning_efforts: efforts(&["low", "high", "max"]),
            context_window: Some(1_048_576),
            // `input_modalities` names `image` among them.
            image_input: Some(true),
            reasoning: Some(true),
            ..unknown_entry("glm-5.3-flash")
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
async fn a_page_without_an_entry_array_fails_the_listing() {
    for body in [
        r#"{"object":"list"}"#,
        r#"{"data":{"id":"model-a"}}"#,
        r#"{"models":{"slug":"model-a"}}"#,
        r#"{"object":"list","data":null,"models":null}"#,
        "[]",
    ] {
        assert!(
            matches!(
                list_one_page(ModelApiProtocol::ChatCompletions, body).await,
                Err(ModelError::Protocol(message)) if message.contains("no models array")
            ),
            "{body}"
        );
    }
}

#[tokio::test]
async fn an_empty_entry_array_lists_no_models() {
    for body in [r#"{"data":[]}"#, r#"{"object":"list","models":[]}"#] {
        let listing = list_one_page(ModelApiProtocol::ChatCompletions, body)
            .await
            .unwrap();
        assert_eq!(listing.skipped, 0);
        assert!(listing.entries.is_empty(), "{body}");
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
async fn google_catalog_normalizes_model_resources_and_follows_page_tokens() {
    for protocol in [
        ModelApiProtocol::GeminiGenerateContent,
        ModelApiProtocol::GeminiInteractions,
    ] {
        let page =
            |value: serde_json::Value| PlannedResponse::status(200, value.to_string().into_bytes());
        let server = FakeHttpServer::start(vec![
            page(json!({"models": [
                {"name":"models/gemini-first","displayName":"First","inputTokenLimit":1000000,"outputTokenLimit":8192,"thinking":true},
                {"name":"models/embed","supportedGenerationMethods":["embedContent","countTokens"]},
                {"name":"models/"}
            ],"nextPageToken":"next +/"})),
            page(json!({"models": [{"name":"models/gemini-next","supportedGenerationMethods":["generateContent","embedContent"]}],
                        "nextPageToken":"next +/"})),
        ]).await;
        let provider = ModelProvider::new(
            ModelEndpoint::parse(&format!("{}/gateway/v1?tenant=unit", server.base_url())).unwrap(),
            Some(ModelCredential::new("api-secret").unwrap()),
        )
        .with_header(ModelHeader::new("x-scope", "example").unwrap())
        .with_header(ModelHeader::new("x-goog-api-key", "obsolete-key").unwrap());
        let listing = ModelCatalogClient::new(provider)
            .unwrap()
            .list(protocol, CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(listing.skipped, 1);
        assert_eq!(
            listing.entries,
            [
                ModelCatalogEntry {
                    display_name: Some("First".into()),
                    context_window: Some(1000000),
                    max_output_tokens: Some(8192),
                    reasoning: Some(true),
                    ..unknown_entry("gemini-first")
                },
                unknown_entry("gemini-next"),
            ]
        );
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, "/v1/gateway/v1/models?tenant=unit");
        assert_eq!(
            requests[1].path,
            "/v1/gateway/v1/models?tenant=unit&pageToken=next+%2B%2F"
        );
        for request in requests {
            assert_eq!(request.headers["x-goog-api-key"], "api-secret");
            assert_eq!(request.headers["x-scope"], "example");
            assert!(!request.headers.contains_key("authorization"));
        }
    }
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

fn efforts(levels: &[&str]) -> Vec<ModelReasoningEffort> {
    levels
        .iter()
        .map(|level| ModelReasoningEffort::parse(*level).unwrap())
        .collect()
}

#[tokio::test]
async fn effort_lists_skip_what_does_not_parse_and_the_first_shape_listing_any_wins() {
    let page = json!({"data": [
        // OpenRouter's `none` and a level off the ladder.
        {"id": "router", "reasoning": {"supported_efforts": ["high", "none", "turbo", 7, "low"], "default_effort": "none"}},
        // Codex's levels win over OpenRouter's.
        {"id": "codex", "supported_reasoning_levels": [{"effort": "high"}], "reasoning": {"supported_efforts": ["low"], "default_effort": "low"}},
        // An Anthropic level is listed only when it says it is supported.
        {"id": "anthropic", "capabilities": {"effort": {"supported": true, "low": {"supported": true}, "max": {"supported": false}, "high": null}}},
        // A null allowlist lists nothing.
        {"id": "open", "reasoning": {"supported_efforts": null, "default_effort": "medium"}},
    ]});
    let listing = list_one_page(ModelApiProtocol::ChatCompletions, &page.to_string())
        .await
        .unwrap();
    let by_id = |id: &str| {
        listing
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .unwrap()
            .clone()
    };
    assert_eq!(
        by_id("router").supported_reasoning_efforts,
        efforts(&["low", "high", "turbo"])
    );
    assert_eq!(by_id("router").default_reasoning_effort, None);
    assert_eq!(
        by_id("codex").supported_reasoning_efforts,
        efforts(&["high"])
    );
    assert_eq!(
        by_id("codex").default_reasoning_effort,
        Some(ModelReasoningEffort::Low)
    );
    assert_eq!(
        by_id("anthropic").supported_reasoning_efforts,
        efforts(&["low"])
    );
    assert_eq!(by_id("anthropic").reasoning, Some(true));
    assert!(by_id("open").supported_reasoning_efforts.is_empty());
    assert_eq!(
        by_id("open").default_reasoning_effort,
        Some(ModelReasoningEffort::Medium)
    );
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
