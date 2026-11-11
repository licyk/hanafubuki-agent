use serde::{Deserialize, Serialize};

use hanafubuki_agent_tool_contracts::ToolProviderCapabilities;

use crate::ModelStructuredOutputCapability;

/// Exact wire protocol used for model sampling and model catalog discovery.
///
/// This is deliberately not inferred from a URL or model name. Compatible
/// providers frequently expose several endpoints at the same base URL, so the
/// caller must select one explicit protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelApiProtocol {
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

impl ModelApiProtocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::ChatCompletions => "chat_completions",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }

    /// Protocol-level capabilities. Model- or endpoint-specific restrictions
    /// may only narrow this snapshot before the Agent freezes a turn.
    pub const fn tool_capabilities(self) -> ToolProviderCapabilities {
        match self {
            Self::Responses => ToolProviderCapabilities {
                function_tools: true,
                strict_json_schema: true,
                maximum_tool_name_bytes: 128,
                namespace_tools: true,
                native_tool_search: true,
                freeform_tools: true,
                hosted_web_search: true,
                parallel_tool_calls: true,
                multimodal_tool_output_transport:
                    hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::NativeToolResult,
                required_named_tool_choice: true,
            },
            Self::ChatCompletions => ToolProviderCapabilities {
                function_tools: true,
                // The protocol of every OpenAI-compatible endpoint, some of
                // which refuse JSON Schema keywords outside the portable set.
                strict_json_schema: false,
                // OpenAI's function name: a-z, A-Z, 0-9, `_` and `-`, at most 64.
                maximum_tool_name_bytes: 64,
                namespace_tools: false,
                native_tool_search: false,
                freeform_tools: false,
                hosted_web_search: false,
                parallel_tool_calls: true,
                multimodal_tool_output_transport:
                    hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::SyntheticUserMessage,
                required_named_tool_choice: true,
            },
            Self::AnthropicMessages => ToolProviderCapabilities {
                function_tools: true,
                strict_json_schema: false,
                // A client tool's name matches `^[a-zA-Z0-9_-]{1,128}$`.
                maximum_tool_name_bytes: 128,
                namespace_tools: false,
                native_tool_search: false,
                freeform_tools: false,
                hosted_web_search: false,
                parallel_tool_calls: true,
                multimodal_tool_output_transport:
                    hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::NativeToolResult,
                required_named_tool_choice: true,
            },
        }
    }

    pub const fn structured_output_capability(self) -> ModelStructuredOutputCapability {
        match self {
            Self::Responses | Self::ChatCompletions => {
                ModelStructuredOutputCapability::NativeJsonSchema
            }
            Self::AnthropicMessages => ModelStructuredOutputCapability::TextOnly,
        }
    }

    /// Whether a session may compact on the provider. A session on another
    /// protocol summarizes locally whatever its policy names.
    pub const fn supports_remote_compaction(self) -> bool {
        match self {
            Self::Responses => true,
            Self::ChatCompletions | Self::AnthropicMessages => false,
        }
    }
}

#[cfg(test)]
mod tests;
