//! Tauri-free model request and streaming contracts.
//!
//! This crate is the wire/mechanism boundary below the Agent state machine. It
//! does not resolve Config credentials, execute tools, or own conversation
//! state. Its transports implement the Responses, Chat Completions, and Anthropic
//! Messages HTTP/SSE protocols without depending on Config or Tauri.

mod anthropic_messages;
mod chat_completions;
mod image_input;
pub mod model;
mod model_catalog;
pub mod port;
pub mod protocol;
pub mod provider;
mod responses;
mod sse;
mod streaming;
mod tolerant_decode;
mod tool_lowering;
mod tool_output_lowering;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use anthropic_messages::AnthropicMessagesModelTransport;
pub use chat_completions::ChatCompletionsModelTransport;
pub use hanafubuki_agent_tool_contracts::{ToolProviderCapabilities, ToolSpec};
pub use image_input::{
    classify_remote_model_error, request_contains_images, sanitize_images_for_unsupported_input,
    ImageSanitizationReport, ModelRouteKey, IMAGE_INPUT_OMITTED_PLACEHOLDER,
};
pub use model::{
    ModelCompactionCapability, ModelCompactionResult, ModelContextRole, ModelEvent,
    ModelImageDetail, ModelImageInputCapability, ModelImageInputPolicy, ModelInputCapabilities,
    ModelInputCapabilityPolicy, ModelInputItem, ModelJsonSchemaOutput, ModelMessagePhase,
    ModelOpaqueCompactionPayload, ModelOutputFormat, ModelOutputItem, ModelReasoningConfig,
    ModelReasoningEffort, ModelRequest, ModelResponseMetadata, ModelStructuredOutputCapability,
    ModelToolArgumentError, ModelToolCall, ModelToolCallOrigin, ModelToolChoice, ModelToolSpec,
    ModelUsage, ModelUserContent, RateLimitMetadata, FALLBACK_TOOL_SEARCH_NAME,
    MAX_MODEL_REASONING_EFFORT_BYTES, MAX_TOOL_ARGUMENT_PREVIEW_BYTES, NATIVE_TOOL_SEARCH_NAME,
};
pub use model_catalog::{ModelCatalogClient, ModelCatalogEntry};
pub use port::{
    ModelError, ModelEventStream, ModelFuture, ModelPort, ModelResult, ModelSession,
    ModelSessionFactory, ModelStream,
};
pub use protocol::ModelApiProtocol;
pub use provider::{
    ModelCredential, ModelEndpoint, ModelHeader, ModelProvider, ModelRetryPolicy, ModelTimeouts,
};
pub use responses::ResponsesModelTransport;
pub use streaming::{bounded_trace_content, MAX_TRACE_CONTENT_BYTES};
pub use tool_lowering::flattened_tool_call_name;
