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

/// A reasoning block a Provider asks to have sent back, unchanged, with the
/// turn that produced it: Anthropic's `thinking` (its whole text and its
/// signature) and `redacted_thinking`. Its `Debug` shows neither.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelReasoningReplay {
    Thinking { thinking: String, signature: String },
    RedactedThinking { data: String },
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
}

impl std::fmt::Debug for ModelReasoningReplay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Thinking { .. } => "ModelReasoningReplay::Thinking([redacted])",
            Self::RedactedThinking { .. } => "ModelReasoningReplay::RedactedThinking([redacted])",
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
