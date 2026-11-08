use super::{ModelOutputItem, ModelResponseMetadata, ModelUsage};
use serde::{Deserialize, Serialize};

/// Incremental events emitted by a model transport.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelEvent {
    ResponseStarted {
        response_id: String,
        metadata: ModelResponseMetadata,
    },
    OutputItemAdded {
        item: ModelOutputItem,
    },
    AssistantTextDelta {
        item_id: String,
        delta: String,
    },
    /// A fragment of the model's own summary of an in-progress reasoning
    /// item. `summary_index` identifies the summary part; a new part starts
    /// a new paragraph, so consumers must not join fragments across parts.
    ReasoningSummaryDelta {
        item_id: String,
        summary_index: u32,
        delta: String,
    },
    OutputItemCompleted {
        item: ModelOutputItem,
    },
    ResponseCompleted {
        response_id: String,
        usage: Option<ModelUsage>,
        end_turn: Option<bool>,
    },
    /// The response stopped before it finished (Responses
    /// `response.incomplete`, for example at the output token limit). The
    /// items already completed stand; an Agent turn keeps the partial output
    /// streamed so far. `reason` is the provider's bounded reason.
    ResponseIncomplete {
        response_id: String,
        usage: Option<ModelUsage>,
        reason: String,
    },
}
