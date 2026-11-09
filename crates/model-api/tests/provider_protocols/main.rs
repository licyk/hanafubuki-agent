#[path = "../support/mod.rs"]
mod support;

use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::{
    AnthropicMessagesModelTransport, ChatCompletionsModelTransport, ModelApiProtocol,
    ModelCatalogClient, ModelContextRole, ModelCredential, ModelEndpoint, ModelError, ModelEvent,
    ModelHeader, ModelImageDetail, ModelInputItem, ModelJsonSchemaOutput, ModelMessagePhase,
    ModelOutputFormat, ModelOutputItem, ModelPort, ModelProvider, ModelReasoningConfig,
    ModelReasoningEffort, ModelRequest, ModelToolChoice, ModelToolSpec, ModelUsage,
    ModelUserContent, ResponsesModelTransport,
};
use hanafubuki_agent_tool_contracts::{
    FreeformToolFormat, FreeformToolSpec, FunctionToolSpec, HostedWebSearchSpec, NamespaceToolSpec,
    ToolName, ToolOutput, ToolOutputContent, ToolSearchSpec, ToolSpec,
};
use serde_json::{json, Value};
use support::{FakeHttpServer, PlannedResponse, ResponseFragment};

fn provider(server: &FakeHttpServer) -> ModelProvider {
    ModelProvider::new(
        ModelEndpoint::parse(server.base_url()).unwrap(),
        Some(ModelCredential::new("provider-secret").unwrap()),
    )
    .with_header(ModelHeader::new("x-provider-scope", "workspace-a").unwrap())
}

fn fixture_request() -> ModelRequest {
    ModelRequest {
        model: "fixture-model".into(),
        instructions: "Use typed tools.".into(),
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
            ModelInputItem::ContextFragment {
                role: ModelContextRole::User,
                kind: "task_plan".into(),
                content: "- [completed] Inspect\n- [in_progress] Implement\n".into(),
            },
            ModelInputItem::user_text("hello"),
            ModelInputItem::ToolCall {
                call_id: "call-old".into(),
                namespace: None,
                name: "list_instances".into(),
                arguments: json!({}),
            },
            ModelInputItem::ToolOutput {
                call_id: "call-old".into(),
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

fn structured_output_format() -> ModelOutputFormat {
    ModelOutputFormat::JsonSchema(ModelJsonSchemaOutput {
        name: "permission_assessment".to_string(),
        description: Some("Review result".to_string()),
        schema: json!({
            "type": "object",
            "properties": {"outcome": {"type": "string", "enum": ["allow", "deny"]}},
            "required": ["outcome"],
            "additionalProperties": false
        }),
        strict: true,
    })
}

async fn collect(transport: &dyn ModelPort) -> Vec<ModelEvent> {
    collect_request(transport, fixture_request()).await
}

async fn collect_request(transport: &dyn ModelPort, request: ModelRequest) -> Vec<ModelEvent> {
    let mut stream = transport
        .stream(request, CancellationToken::default())
        .await
        .unwrap();
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.unwrap());
    }
    events
}

mod anthropic;
mod chat;
mod cross_protocol;
mod reasoning;
mod responses;
mod tolerance;
