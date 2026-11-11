use super::{ModelMessagePhase, ModelOpaqueCompactionPayload};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelImageDetail {
    Low,
    High,
    #[default]
    Auto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelUserContent {
    InputText {
        text: String,
    },
    InputImage {
        mime_type: String,
        data: Vec<u8>,
        detail: ModelImageDetail,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInputCapabilities {
    pub image_input: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelImageInputPolicy {
    #[default]
    Auto,
    Disabled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInputCapabilityPolicy {
    pub image_input: ModelImageInputPolicy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelImageInputCapability {
    #[default]
    Unknown,
    Supported,
    Unsupported,
}

impl ModelInputCapabilityPolicy {
    pub fn effective(self, observed: ModelImageInputCapability) -> ModelInputCapabilities {
        ModelInputCapabilities {
            image_input: self.image_input != ModelImageInputPolicy::Disabled
                && observed != ModelImageInputCapability::Unsupported,
        }
    }
}

pub const NATIVE_TOOL_SEARCH_NAME: &str = "__hanafubuki_native_tool_search";
pub const FALLBACK_TOOL_SEARCH_NAME: &str = "tool_search";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelContextRole {
    Developer,
    User,
}

/// Canonical model-visible conversation item used at the Agent/model boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelInputItem {
    /// Context the Agent rendered in full; a provider sends its role and
    /// content as they are.
    ContextFragment {
        role: ModelContextRole,
        /// What the fragment renders (a context section kind or a kernel
        /// item), for diagnostics and tests; never sent to a provider.
        kind: String,
        content: String,
    },
    UserMessage {
        content: Vec<ModelUserContent>,
    },
    AssistantMessage {
        phase: Option<ModelMessagePhase>,
        text: String,
    },
    /// A reasoning block of the current turn that the provider asks to have
    /// sent back with the assistant message after it; only Anthropic
    /// Messages sends it, and only with manual thinking.
    Reasoning {
        replay: super::ModelReasoningReplay,
    },
    ToolCall {
        call_id: String,
        /// The native namespace of the call, replayed as the provider sent
        /// it ([`ModelToolCall::namespace`](crate::ModelToolCall)).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        name: String,
        arguments: Value,
    },
    CustomToolCall {
        call_id: String,
        name: String,
        input: String,
    },
    ToolOutput {
        call_id: String,
        output: hanafubuki_agent_tool_contracts::ToolOutput,
    },
    CustomToolOutput {
        call_id: String,
        output: hanafubuki_agent_tool_contracts::ToolOutput,
    },
    ToolSearchCall {
        call_id: String,
        arguments: Value,
    },
    ToolSearchOutput {
        call_id: String,
        tools: Vec<hanafubuki_agent_tool_contracts::ToolSpec>,
    },
    ContinuationContext {
        provider_id: String,
        hook_id: String,
        generation: [u8; 32],
        content: String,
    },
    CompactionSummary {
        summary: String,
        source_revision: u64,
        compacted_item_count: usize,
    },
    CompactionTrigger,
    Compaction {
        payload: ModelOpaqueCompactionPayload,
    },
}

impl ModelInputItem {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self::UserMessage {
            content: vec![ModelUserContent::InputText { text: text.into() }],
        }
    }
}

pub(crate) fn image_data_url(mime_type: &str, data: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "data:{mime_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

pub(crate) fn image_detail_name(detail: ModelImageDetail) -> &'static str {
    match detail {
        ModelImageDetail::Low => "low",
        ModelImageDetail::High => "high",
        ModelImageDetail::Auto => "auto",
    }
}

pub(crate) fn continuation_context_text(
    provider_id: &str,
    hook_id: &str,
    generation: &[u8; 32],
    content: &str,
) -> String {
    let generation = generation
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "[Trusted continuation context; provider {provider_id}; hook {hook_id}; generation {generation}]\n{content}"
    )
}

pub(crate) fn compaction_summary_text(
    summary: &str,
    _source_revision: u64,
    _compacted_item_count: usize,
) -> String {
    summary.to_string()
}
