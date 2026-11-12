use hanafubuki_agent_foundation::agent_settings::ModelReasoningEffort;
use serde::{Deserialize, Serialize};

const MAX_OPAQUE_COMPACTION_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelReasoningConfig {
    pub effort: ModelReasoningEffort,
    /// The largest output the model is known to give, under which
    /// Anthropic Messages sizes its thinking budget and `max_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
}

/// The most bytes of one reasoning block kept to be sent back; a larger
/// block is not sent back.
pub const MAX_MODEL_REASONING_REPLAY_BYTES: usize = 1024 * 1024;

/// A reasoning block a Provider asks to have sent back, unchanged: Anthropic's
/// `thinking` (its whole text and its signature) and `redacted_thinking`,
/// sent back within the turn that produced them, and Chat Completions'
/// `reasoning_content` (DeepSeek's thinking mode), sent back from every turn.
/// Its `Debug` shows none of them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelReasoningReplay {
    Thinking {
        thinking: String,
        signature: String,
    },
    RedactedThinking {
        data: String,
    },
    /// The reasoning text of a Chat Completions response, which goes back as
    /// the `reasoning_content` of the assistant message it preceded.
    ReasoningContent {
        text: String,
    },
}

impl ModelReasoningReplay {
    /// Anthropic's `thinking` block, when it carries a signature and fits
    /// the bound.
    pub(crate) fn thinking(thinking: String, signature: String) -> Option<Self> {
        (!signature.is_empty()
            && thinking.len().saturating_add(signature.len()) <= MAX_MODEL_REASONING_REPLAY_BYTES)
            .then_some(Self::Thinking {
                thinking,
                signature,
            })
    }

    /// Anthropic's `redacted_thinking` block, when it carries data and fits
    /// the bound.
    pub(crate) fn redacted_thinking(data: String) -> Option<Self> {
        (!data.is_empty() && data.len() <= MAX_MODEL_REASONING_REPLAY_BYTES)
            .then_some(Self::RedactedThinking { data })
    }

    /// Chat Completions' `reasoning_content` text, when it has any and fits
    /// the bound.
    pub fn reasoning_content(text: String) -> Option<Self> {
        (!text.is_empty() && text.len() <= MAX_MODEL_REASONING_REPLAY_BYTES)
            .then_some(Self::ReasoningContent { text })
    }

    /// Whether the Provider takes this block back with every later request
    /// that still holds the message it preceded, not only within its own
    /// turn: `reasoning_content`. Such a block is kept with the history,
    /// since a resumed session still sends it; a turn's block (Anthropic's)
    /// is kept in memory only.
    pub fn outlives_its_turn(&self) -> bool {
        matches!(self, Self::ReasoningContent { .. })
    }

    /// The bytes of text the block sends back.
    pub fn text_len(&self) -> usize {
        match self {
            Self::Thinking {
                thinking,
                signature,
            } => thinking.len().saturating_add(signature.len()),
            Self::RedactedThinking { data } => data.len(),
            Self::ReasoningContent { text } => text.len(),
        }
    }
}

impl std::fmt::Debug for ModelReasoningReplay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Thinking { .. } => "ModelReasoningReplay::Thinking([redacted])",
            Self::RedactedThinking { .. } => "ModelReasoningReplay::RedactedThinking([redacted])",
            Self::ReasoningContent { .. } => "ModelReasoningReplay::ReasoningContent([redacted])",
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ModelOpaqueCompactionPayload(String);

impl ModelOpaqueCompactionPayload {
    pub fn parse(value: String) -> Result<Self, String> {
        if value.is_empty() || value.len() > MAX_OPAQUE_COMPACTION_PAYLOAD_BYTES {
            return Err(format!(
                "opaque compaction payload must contain 1 to {MAX_OPAQUE_COMPACTION_PAYLOAD_BYTES} bytes"
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ModelOpaqueCompactionPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ModelOpaqueCompactionPayload([redacted])")
    }
}

impl<'de> Deserialize<'de> for ModelOpaqueCompactionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCompactionCapability {
    #[default]
    LocalSummary,
    ResponsesCompact,
    ResponsesCompactV2,
}
