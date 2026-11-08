use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    validate_description, validate_json_schema, ToolContractError, ToolName,
    MAX_FREEFORM_GRAMMAR_BYTES,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExposure {
    #[default]
    Direct,
    Deferred,
    DirectModelOnly,
    Hidden,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionClass {
    ConcurrentRead,
    #[default]
    Exclusive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionToolSpec {
    pub name: ToolName,
    pub description: String,
    pub input_schema: Value,
    pub strict: bool,
    pub deferred: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceToolSpec {
    pub name: ToolName,
    pub description: String,
    pub tools: Vec<FunctionToolSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSearchSpec {
    pub execution: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedWebSearchSpec {
    pub external_web_access: Option<bool>,
    pub indexed_web_access: Option<bool>,
    pub allowed_domains: Option<Vec<String>>,
    pub search_context_size: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeformToolFormat {
    pub format_type: String,
    pub syntax: String,
    pub definition: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeformToolSpec {
    pub name: ToolName,
    pub description: String,
    pub format: FreeformToolFormat,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolSpec {
    Function(FunctionToolSpec),
    Namespace(NamespaceToolSpec),
    ToolSearch(ToolSearchSpec),
    HostedWebSearch(HostedWebSearchSpec),
    Freeform(FreeformToolSpec),
}

impl ToolSpec {
    pub fn validate(&self) -> Result<(), ToolContractError> {
        validate_description(self.description())?;
        match self {
            Self::Function(spec) => validate_json_schema(&spec.input_schema),
            Self::Namespace(spec) => {
                for tool in &spec.tools {
                    validate_description(&tool.description)?;
                    validate_json_schema(&tool.input_schema)?;
                }
                Ok(())
            }
            Self::ToolSearch(spec) => validate_json_schema(&spec.parameters),
            Self::Freeform(spec) if spec.format.definition.len() > MAX_FREEFORM_GRAMMAR_BYTES => {
                Err(ToolContractError::GrammarTooLarge)
            }
            _ => Ok(()),
        }
    }

    pub fn name(&self) -> Option<&ToolName> {
        Some(match self {
            Self::Function(spec) => &spec.name,
            Self::Namespace(spec) => &spec.name,
            Self::ToolSearch(_) => return None,
            Self::HostedWebSearch(_) => return None,
            Self::Freeform(spec) => &spec.name,
        })
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Function(spec) => &spec.description,
            Self::Namespace(spec) => &spec.description,
            Self::ToolSearch(spec) => &spec.description,
            Self::HostedWebSearch(_) => "Hosted web search",
            Self::Freeform(spec) => &spec.description,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultimodalToolOutputTransport {
    Unsupported,
    NativeToolResult,
    SyntheticUserMessage,
}

impl MultimodalToolOutputTransport {
    pub const fn is_supported(self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolProviderCapabilities {
    pub function_tools: bool,
    /// The protocol takes a tool's JSON Schema whole, constraints such as
    /// `minimum`, `maxLength`, `pattern`, `format` and `default` included.
    /// Without it the tool plan lowers every schema to the portable keywords.
    pub strict_json_schema: bool,
    /// The longest tool name the protocol takes, counted over the flattened
    /// `namespace__name` when the tool sits in a native namespace. The tool
    /// plan fits every wire name to it.
    pub maximum_tool_name_bytes: usize,
    pub namespace_tools: bool,
    pub native_tool_search: bool,
    pub freeform_tools: bool,
    pub hosted_web_search: bool,
    pub parallel_tool_calls: bool,
    pub multimodal_tool_output_transport: MultimodalToolOutputTransport,
    pub required_named_tool_choice: bool,
}
