use super::{ModelOpaqueCompactionPayload, NATIVE_TOOL_SEARCH_NAME};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelToolCall {
    pub item_id: String,
    pub call_id: String,
    /// The native namespace the provider called the tool in, as it sent
    /// it; `None` for a top-level name. Together with `name` it is the wire
    /// name the Agent's tool plan resolves, never split or joined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub name: String,
    /// The parsed arguments. `{}` when [`Self::argument_error`] is set.
    pub arguments: Value,
    /// Set when the provider delivered arguments the Agent cannot use (not
    /// JSON, or over the argument size bound). The call still pairs with an
    /// output: the kernel answers it with a model-visible error instead of
    /// running the tool. Never persisted: history keeps `arguments`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_error: Option<ModelToolArgumentError>,
}

/// Why a tool call's raw arguments could not be used, with a bounded
/// preview of what the model sent, for the model-visible error result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelToolArgumentError {
    /// The parser's or bound's message.
    pub reason: String,
    /// The start of the raw arguments, at most
    /// [`MAX_TOOL_ARGUMENT_PREVIEW_BYTES`], cut on a UTF-8 boundary.
    pub preview: String,
}

/// Bound of [`ModelToolArgumentError::preview`].
pub const MAX_TOOL_ARGUMENT_PREVIEW_BYTES: usize = 512;

impl ModelToolArgumentError {
    /// Builds the error, cutting `raw` to the preview bound on a UTF-8
    /// boundary.
    pub fn new(reason: impl Into<String>, raw: &str) -> Self {
        let mut end = raw.len().min(MAX_TOOL_ARGUMENT_PREVIEW_BYTES);
        while !raw.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            reason: reason.into(),
            preview: raw[..end].to_string(),
        }
    }
}

impl ModelToolCall {
    pub fn origin(&self) -> ModelToolCallOrigin {
        if self.name == NATIVE_TOOL_SEARCH_NAME {
            ModelToolCallOrigin::ToolSearch
        } else if self.arguments.is_string() {
            ModelToolCallOrigin::Freeform
        } else {
            ModelToolCallOrigin::Function
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelToolCallOrigin {
    #[default]
    Function,
    Freeform,
    ToolSearch,
}

/// Semantic role of an assistant message within an Agent turn.
///
/// Providers that expose a native phase map it directly. Protocols without
/// that concept leave the containing `Option` empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelMessagePhase {
    Commentary,
    FinalAnswer,
}

/// Which text of a reasoning item a [`super::ModelEvent::ReasoningDelta`]
/// carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelReasoningTextKind {
    /// The model's own summary of its reasoning.
    Summary,
    /// The reasoning text itself, as open-weight models and their servers
    /// stream it.
    Raw,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelOutputItem {
    AssistantMessage {
        item_id: String,
        phase: Option<ModelMessagePhase>,
        text: String,
    },
    ToolCall {
        call: ModelToolCall,
    },
    /// A reasoning item with its summary parts and its raw reasoning text
    /// parts (`content`), either of which a provider may leave empty, and
    /// the block the provider asks to have sent back with the turn, if any,
    /// which is never serialized.
    Reasoning {
        item_id: String,
        summary: Vec<String>,
        content: Vec<String>,
        #[serde(skip)]
        replay: Option<super::ModelReasoningReplay>,
    },
    Compaction {
        item_id: String,
        payload: ModelOpaqueCompactionPayload,
    },
    Other {
        item_id: Option<String>,
        item_type: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}
