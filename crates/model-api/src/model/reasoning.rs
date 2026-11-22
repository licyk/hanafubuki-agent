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

/// Byte bound for Anthropic replay blocks and Chat Completions reasoning text.
pub const MAX_MODEL_REASONING_REPLAY_BYTES: usize = 1024 * 1024;

/// Ordered pieces of a GenerateContent candidate. A function's arguments and
/// name already live in its canonical tool call, keyed by the local call id.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GeminiReplayPart {
    Text {
        text: String,
        thought: bool,
        signature: Option<String>,
    },
    Call {
        call_id: String,
        provider_id: Option<String>,
        signature: Option<String>,
    },
}

impl std::fmt::Debug for GeminiReplayPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GeminiReplayPart([redacted])")
    }
}

/// Provider-owned data for replay. Anthropic blocks are turn-local. Chat
/// Completions reasoning and native Gemini signatures survive restoration.
/// Debug output omits their contents.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelReasoningReplay {
    GeminiGenerate {
        route: crate::ModelRouteKey,
        parts: Vec<GeminiReplayPart>,
    },
    GeminiInteractionsThought {
        route: crate::ModelRouteKey,
        signature: String,
        /// Original signed summary blocks, including provider extensions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<Vec<serde_json::Value>>,
    },
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

    /// Whether a later turn or restored session may need this block.
    pub fn outlives_its_turn(&self) -> bool {
        matches!(
            self,
            Self::ReasoningContent { .. }
                | Self::GeminiGenerate { .. }
                | Self::GeminiInteractionsThought { .. }
        )
    }

    /// GenerateContent records the signed layout of a candidate after its
    /// visible parts have streamed. This metadata has no thinking UI.
    pub fn is_part_layout(&self) -> bool {
        matches!(self, Self::GeminiGenerate { .. })
    }

    /// Whether this replay can be restored from persistent history. Signed
    /// Gemini parts must survive restarts intact to continue tool rounds.
    pub fn is_persistable(&self) -> bool {
        match self {
            Self::GeminiGenerate { .. } | Self::GeminiInteractionsThought { .. } => true,
            Self::ReasoningContent { text } => text.len() <= MAX_MODEL_REASONING_REPLAY_BYTES,
            Self::Thinking { .. } | Self::RedactedThinking { .. } => false,
        }
    }

    /// Text this replay adds to the canonical messages' context estimate.
    /// Images have a separate estimate in the projected request.
    pub fn text_len(&self) -> usize {
        match self {
            Self::Thinking {
                thinking,
                signature,
            } => thinking.len().saturating_add(signature.len()),
            Self::GeminiGenerate { parts, .. } => parts.iter().fold(0usize, |total, part| {
                let (base, signature) = match part {
                    GeminiReplayPart::Text {
                        text,
                        thought,
                        signature,
                    } => (if *thought { text.len() } else { 0 }, signature),
                    GeminiReplayPart::Call { signature, .. } => (0, signature),
                };
                total
                    .saturating_add(base)
                    .saturating_add(signature.as_ref().map_or(0, String::len))
            }),
            Self::GeminiInteractionsThought {
                signature, summary, ..
            } => summary
                .iter()
                .flatten()
                .fold(signature.len(), |total, block| {
                    total.saturating_add(block["text"].as_str().map_or(0, str::len))
                }),
            Self::RedactedThinking { data } => data.len(),
            Self::ReasoningContent { text } => text.len(),
        }
    }
}

impl std::fmt::Debug for ModelReasoningReplay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::GeminiGenerate { .. } => "ModelReasoningReplay::GeminiGenerate([redacted])",
            Self::GeminiInteractionsThought { .. } => {
                "ModelReasoningReplay::GeminiInteractionsThought([redacted])"
            }
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
