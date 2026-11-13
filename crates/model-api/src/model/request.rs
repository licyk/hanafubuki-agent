use super::{ModelInputItem, ModelReasoningConfig, ModelUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A model-visible tool definition. Arguments remain JSON only at this
/// controlled wire boundary and must be validated into typed Agent calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Whether the provider may enforce its strict JSON-schema subset for this
    /// tool. Dynamic/MCP schemas default to false until locally proven strict.
    pub strict: bool,
}

impl TryFrom<ModelToolSpec> for hanafubuki_agent_tool_contracts::ToolSpec {
    type Error = hanafubuki_agent_tool_contracts::ToolContractError;

    fn try_from(spec: ModelToolSpec) -> Result<Self, Self::Error> {
        Ok(Self::Function(
            hanafubuki_agent_tool_contracts::FunctionToolSpec {
                name: hanafubuki_agent_tool_contracts::ToolName::parse(spec.name)?,
                description: spec.description,
                input_schema: spec.input_schema,
                strict: spec.strict,
                deferred: false,
            },
        ))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ModelToolChoice {
    #[default]
    Auto,
    None,
    Required {
        name: String,
    },
}

/// One way of asking a model for a JSON object (a rung), best first: the
/// greater of two is the further down the fallback chain.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ModelStructuredOutputCapability {
    /// The protocol's own JSON-schema output (`text.format`,
    /// `response_format`).
    NativeJsonSchema,
    /// A forced call of one tool whose arguments are the object.
    RequiredTool,
    /// Plain text, parsed afterwards.
    #[default]
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelJsonSchemaOutput {
    pub name: String,
    pub description: Option<String>,
    pub schema: Value,
    pub strict: bool,
}

impl ModelJsonSchemaOutput {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty()
            || self.name.len() > 64
            || !self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(
                "JSON schema output name must contain 1 to 64 ASCII letters, digits, '_' or '-'"
                    .to_string(),
            );
        }
        if self
            .description
            .as_ref()
            .is_some_and(|description| description.len() > 1024)
        {
            return Err("JSON schema output description exceeds 1024 bytes".to_string());
        }
        if !self.schema.is_object() {
            return Err("JSON schema output schema must be an object".to_string());
        }
        let schema_bytes = serde_json::to_vec(&self.schema)
            .map_err(|_| "JSON schema output schema could not be encoded".to_string())?
            .len();
        if schema_bytes > 64 * 1024 {
            return Err("JSON schema output schema exceeds 65536 bytes".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ModelOutputFormat {
    JsonSchema(ModelJsonSchemaOutput),
}

/// Provider-neutral request for one model sampling attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequest {
    pub model: String,
    pub instructions: String,
    pub input: Vec<ModelInputItem>,
    pub tools: Vec<hanafubuki_agent_tool_contracts::ToolSpec>,
    pub tool_choice: ModelToolChoice,
    pub parallel_tool_calls: bool,
    pub reasoning: Option<ModelReasoningConfig>,
    pub output_format: Option<ModelOutputFormat>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCompactionResult {
    pub replacement_history: Vec<ModelInputItem>,
    pub usage: Option<ModelUsage>,
}
