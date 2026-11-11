//! Tauri-free model request and streaming contracts.
//!
//! This crate is the wire/mechanism boundary below the Agent state machine. It
//! does not resolve Config credentials, execute tools, or own conversation
//! state. Its transports implement the Responses, Chat Completions, and Anthropic
//! Messages HTTP/SSE protocols without depending on Config or Tauri.

mod anthropic_messages;
mod chat_completions;
mod context_overflow;
mod image_input;
pub mod model;
mod model_catalog;
mod model_metadata;
pub mod port;
pub mod protocol;
pub mod provider;
mod reasoning_effort;
mod reasoning_refusal;
mod responses;
mod route;
mod sse;
mod streaming;
mod tolerant_decode;
mod tool_lowering;
mod tool_output_lowering;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use anthropic_messages::AnthropicMessagesModelTransport;
pub use chat_completions::ChatCompletionsModelTransport;
pub use context_overflow::{context_limit_in, PLAUSIBLE_CONTEXT_WINDOW};
pub use hanafubuki_agent_foundation::trace_content::{
    bounded_trace_content, MAX_TRACE_CONTENT_BYTES,
};
pub use hanafubuki_agent_tool_contracts::{ToolProviderCapabilities, ToolSpec};
pub use image_input::{
    classify_remote_model_error, request_contains_images, sanitize_images_for_unsupported_input,
    ImageSanitizationReport, IMAGE_INPUT_OMITTED_PLACEHOLDER,
};
pub use model::{
    ModelCompactionCapability, ModelCompactionResult, ModelContextRole, ModelEvent,
    ModelImageDetail, ModelImageInputCapability, ModelImageInputPolicy, ModelInputCapabilities,
    ModelInputCapabilityPolicy, ModelInputItem, ModelJsonSchemaOutput, ModelMessagePhase,
    ModelOpaqueCompactionPayload, ModelOutputFormat, ModelOutputItem, ModelReasoningConfig,
    ModelReasoningEffort, ModelReasoningReplay, ModelReasoningTextKind, ModelRequest,
    ModelResponseMetadata, ModelStructuredOutputCapability, ModelToolArgumentError, ModelToolCall,
    ModelToolCallOrigin, ModelToolChoice, ModelToolSpec, ModelUsage, ModelUserContent,
    RateLimitMetadata, FALLBACK_TOOL_SEARCH_NAME, MAX_MODEL_REASONING_EFFORT_BYTES,
    MAX_MODEL_REASONING_REPLAY_BYTES, MAX_TOOL_ARGUMENT_PREVIEW_BYTES, NATIVE_TOOL_SEARCH_NAME,
};
pub use model_catalog::{ModelCatalogClient, ModelCatalogEntry, ModelCatalogListing};
pub use model_metadata::{ModelMetadata, ModelMetadataSource, ModelReasoningSupport, Sourced};
pub use port::{
    ModelError, ModelEventStream, ModelFuture, ModelPort, ModelResult, ModelSession,
    ModelSessionFactory, ModelStream,
};
pub use protocol::ModelApiProtocol;
pub use provider::{
    ModelCredential, ModelEndpoint, ModelHeader, ModelProvider, ModelRetryPolicy, ModelTimeouts,
};
pub use reasoning_effort::{reasoning_effort_rank, request_reasoning_effort};
pub use responses::ResponsesModelTransport;
pub use route::{ModelEndpointKey, ModelRouteKey};
pub use tool_lowering::flattened_tool_call_name;
