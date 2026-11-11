mod event;
mod input;
mod output;
mod reasoning;
mod request;
mod usage;

pub use event::ModelEvent;
/// Config persists the reasoning effort, so it is defined below both.
pub use hanafubuki_agent_foundation::agent_settings::{
    ModelReasoningEffort, MAX_MODEL_REASONING_EFFORT_BYTES,
};
pub use input::{
    ModelContextRole, ModelImageDetail, ModelImageInputCapability, ModelImageInputPolicy,
    ModelInputCapabilities, ModelInputCapabilityPolicy, ModelInputItem, ModelUserContent,
    FALLBACK_TOOL_SEARCH_NAME, NATIVE_TOOL_SEARCH_NAME,
};
pub use output::{
    ModelMessagePhase, ModelOutputItem, ModelReasoningTextKind, ModelToolArgumentError,
    ModelToolCall, ModelToolCallOrigin, ModelUsage, MAX_TOOL_ARGUMENT_PREVIEW_BYTES,
};
pub use reasoning::{
    ModelCompactionCapability, ModelOpaqueCompactionPayload, ModelReasoningConfig,
    ModelReasoningReplay, MAX_MODEL_REASONING_REPLAY_BYTES,
};
pub use request::{
    ModelCompactionResult, ModelJsonSchemaOutput, ModelOutputFormat, ModelRequest,
    ModelStructuredOutputCapability, ModelToolChoice, ModelToolSpec,
};
pub use usage::{ModelResponseMetadata, RateLimitMetadata};

pub(crate) use input::{
    compaction_summary_text, continuation_context_text, image_data_url, image_detail_name,
};
