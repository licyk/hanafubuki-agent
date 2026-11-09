#[path = "../support/mod.rs"]
mod support;

use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::{
    ModelCompactionCapability, ModelContextRole, ModelCredential, ModelEndpoint, ModelError,
    ModelEvent, ModelHeader, ModelInputItem, ModelMessagePhase, ModelOpaqueCompactionPayload,
    ModelOutputItem, ModelPort, ModelProvider, ModelReasoningConfig, ModelReasoningEffort,
    ModelReasoningTextKind, ModelRequest, ModelResponseMetadata, ModelRetryPolicy, ModelTimeouts,
    ModelToolSpec, ModelUsage, RateLimitMetadata, ResponsesModelTransport,
};
use serde_json::{json, Value};
use support::{FakeHttpServer, PlannedResponse, ResponseFragment};

fn streaming_payload(text: &str) -> Vec<u8> {
    format!(
        concat!(
            "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-1\"}}}}\r\n\r\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"message\",\"id\":\"msg-1\",\"phase\":\"final_answer\"}}}}\r\n\r\n",
            "data: {{\"type\":\"response.output_text.delta\",\"item_id\":\"msg-1\",\"delta\":\"{text}\"}}\r\n\r\n",
            "data: {{\"type\":\"response.output_text.done\",\"item_id\":\"msg-1\",\"text\":\"{text}\"}}\r\n\r\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"message\",\"id\":\"msg-1\",\"phase\":\"final_answer\",\"content\":[{{\"type\":\"output_text\",\"text\":\"{text}\"}}]}}}}\r\n\r\n",
            "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-1\",\"usage\":{{\"input_tokens\":7,\"output_tokens\":3}},\"end_turn\":true}}}}\r\n\r\n",
            "data: [DONE]\r\n\r\n"
        ),
        text = text
    )
    .into_bytes()
}

fn tool_call_payload() -> Vec<u8> {
    concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-tool\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"comment\",\"phase\":\"commentary\"}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"comment\",\"delta\":\"Checking\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"comment\",\"phase\":\"commentary\",\"content\":[{\"type\":\"output_text\",\"text\":\"Checking\"}]}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"tool-item\"}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"id\":\"tool-item\",\"call_id\":\"call-tool\",\"name\":\"list_instances\",\"arguments\":\"{}\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-tool\",\"end_turn\":false}}\n\n"
    )
    .as_bytes()
    .to_vec()
}

fn fixture_request() -> ModelRequest {
    ModelRequest {
        model: "fixture-model".into(),
        instructions: "Use only typed tools.".into(),
        input: vec![
            ModelInputItem::ContextFragment {
                role: ModelContextRole::Developer,
                kind: "skill_catalog".into(),
                content: "catalog metadata".into(),
            },
            ModelInputItem::ContextFragment {
                role: ModelContextRole::User,
                kind: "application_ui_context".into(),
                content: r#"{"schemaVersion":1,"page":{"kind":"instance_chooser"}}"#.into(),
            },
            ModelInputItem::ContextFragment {
                role: ModelContextRole::User,
                kind: "skill_instructions".into(),
                content: "untrusted workflow".into(),
            },
            ModelInputItem::user_text("hello"),
            ModelInputItem::AssistantMessage {
                phase: Some(ModelMessagePhase::Commentary),
                text: "checking".into(),
            },
            ModelInputItem::ToolCall {
                call_id: "call-1".into(),
                namespace: None,
                name: "list_instances".into(),
                arguments: json!({}),
            },
            ModelInputItem::ToolOutput {
                call_id: "call-1".into(),
                output: json!({"instances": []}).into(),
            },
        ],
        tools: vec![ModelToolSpec {
            strict: true,
            name: "list_instances".into(),
            description: "List instances".into(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        }
        .try_into()
        .unwrap()],
        tool_choice: hanafubuki_agent_model_api::ModelToolChoice::Auto,
        parallel_tool_calls: false,
        reasoning: None,
        output_format: None,
    }
}

fn provider(server: &FakeHttpServer) -> ModelProvider {
    ModelProvider::new(
        ModelEndpoint::parse(server.base_url()).unwrap(),
        Some(ModelCredential::new("credential-secret").unwrap()),
    )
    .with_header(ModelHeader::new("x-workspace-key", "header-secret").unwrap())
    .with_retry_policy(
        ModelRetryPolicy::new(3, Duration::from_millis(1), Duration::from_millis(2)).unwrap(),
    )
    .with_timeouts(
        ModelTimeouts::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_millis(200),
        )
        .unwrap(),
    )
}

#[test]
fn provider_contract_rejects_invalid_values_and_redacts_debug_secrets() {
    assert!(ModelEndpoint::parse("https://[::1").is_err());
    assert!(ModelHeader::new("authorization", "override").is_err());
    assert!(ModelRetryPolicy::new(11, Duration::from_millis(1), Duration::from_secs(1)).is_err());

    let endpoint = ModelEndpoint::parse("https://example.com/v1?api-key=query-secret").unwrap();
    let header = ModelHeader::new("x-provider-scope", "header-secret").unwrap();
    assert!(!format!("{endpoint:?}").contains("query-secret"));
    assert!(!format!("{header:?}").contains("header-secret"));
}

async fn collect(transport: &ResponsesModelTransport) -> Result<Vec<ModelEvent>, ModelError> {
    let mut stream = transport
        .stream(fixture_request(), CancellationToken::default())
        .await?;
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event?);
    }
    Ok(events)
}

fn compact_request() -> ModelRequest {
    ModelRequest {
        model: "fixture-model".into(),
        instructions: "Keep the exact base instructions.".into(),
        input: vec![
            ModelInputItem::user_text("hello"),
            ModelInputItem::AssistantMessage {
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: "world".into(),
            },
        ],
        tools: fixture_request().tools,
        tool_choice: hanafubuki_agent_model_api::ModelToolChoice::None,
        parallel_tool_calls: true,
        reasoning: None,
        output_format: None,
    }
}

mod compaction;
mod errors;
mod retry;
mod streaming;
mod timeouts;
mod tolerance;
