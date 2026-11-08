use serde::{Deserialize, Serialize};

pub const MAX_TOOL_IDENTIFIER_BYTES: usize = 128;
pub const MAX_TOOL_NAMESPACE_DEPTH: usize = 8;

macro_rules! bounded_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, ToolContractError> {
                let value = value.into();
                validate_identifier(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

bounded_id!(ToolProviderId);
bounded_id!(ToolId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolGeneration(pub [u8; 32]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolCatalogRevision(pub [u8; 32]);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolName {
    namespace: Vec<String>,
    local_name: String,
}

impl ToolName {
    pub fn parse(local_name: impl Into<String>) -> Result<Self, ToolContractError> {
        Self::namespaced(Vec::new(), local_name)
    }

    pub fn namespaced(
        namespace: Vec<String>,
        local_name: impl Into<String>,
    ) -> Result<Self, ToolContractError> {
        if namespace.len() > MAX_TOOL_NAMESPACE_DEPTH {
            return Err(ToolContractError::NamespaceTooDeep);
        }
        for segment in &namespace {
            validate_model_tool_identifier(segment)?;
        }
        let local_name = local_name.into();
        validate_model_tool_identifier(&local_name)?;
        Ok(Self {
            namespace,
            local_name,
        })
    }

    pub fn namespace(&self) -> &[String] {
        &self.namespace
    }

    pub fn local_name(&self) -> &str {
        &self.local_name
    }

    pub fn qualified(&self) -> String {
        if self.namespace.is_empty() {
            self.local_name.clone()
        } else {
            format!("{}::{}", self.namespace.join("::"), self.local_name)
        }
    }

    pub fn flattened_alias(&self) -> String {
        if self.namespace.is_empty() {
            return self.local_name.clone();
        }
        format!("{}__{}", self.namespace.join("__"), self.local_name)
    }
}

fn validate_identifier(value: &str) -> Result<(), ToolContractError> {
    if value.is_empty()
        || value.len() > MAX_TOOL_IDENTIFIER_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ToolContractError::InvalidIdentifier);
    }
    Ok(())
}

fn validate_model_tool_identifier(value: &str) -> Result<(), ToolContractError> {
    if value.is_empty()
        || value.len() > MAX_TOOL_IDENTIFIER_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ToolContractError::InvalidIdentifier);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolContractError {
    InvalidIdentifier,
    NamespaceTooDeep,
    DescriptionTooLong,
    SchemaTooLarge,
    SchemaTooDeep,
    GrammarTooLarge,
    TooManyOutputItems,
    OutputTooLarge,
    InvalidMimeType,
    InvalidExternalUrl,
}

impl std::fmt::Display for ToolContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentifier => "tool identifier is invalid",
            Self::NamespaceTooDeep => "tool namespace is too deep",
            Self::DescriptionTooLong => "tool description is too long",
            Self::SchemaTooLarge => "tool schema is too large",
            Self::SchemaTooDeep => "tool schema is too deep",
            Self::GrammarTooLarge => "freeform grammar is too large",
            Self::TooManyOutputItems => "tool output has too many content items",
            Self::OutputTooLarge => "tool output is too large",
            Self::InvalidMimeType => "tool output MIME type is invalid",
            Self::InvalidExternalUrl => "tool output URL is invalid",
        })
    }
}

impl std::error::Error for ToolContractError {}
