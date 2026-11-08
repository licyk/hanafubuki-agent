use std::fmt;

use serde_json::{Map, Value};

/// A tool call that did not run. Both outcomes are reported to the model by
/// the kernel; `Cancelled` also ends the turn's tool step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentToolError {
    Cancelled,
    Failed(String),
}

impl fmt::Display for AgentToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("agent tool cancelled"),
            Self::Failed(message) => write!(formatter, "agent tool failed: {message}"),
        }
    }
}

impl std::error::Error for AgentToolError {}

impl From<ToolCallValidationError> for AgentToolError {
    fn from(error: ToolCallValidationError) -> Self {
        Self::Failed(error.to_string())
    }
}

/// Model-visible argument errors. The texts are part of what the model reads
/// back, so they stay stable across tools.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolCallValidationError {
    UnknownTool(String),
    ArgumentsMustBeObject,
    UnexpectedArgument {
        name: String,
        accepted: String,
    },
    MissingArgument(&'static str),
    /// An identifier argument, named here, that is not 1 to 256 characters.
    InvalidIdentifier(&'static str),
    InvalidArgument(&'static str),
    /// A field the decoder could not use, named by its path in the
    /// arguments (`steps[1].status`), with the decoder's reason.
    InvalidArgumentAt {
        path: String,
        reason: String,
    },
}

impl fmt::Display for ToolCallValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTool(name) => write!(formatter, "unknown tool {name}"),
            Self::ArgumentsMustBeObject => formatter.write_str("tool arguments must be an object"),
            Self::UnexpectedArgument { name, accepted } => write!(
                formatter,
                "unexpected tool argument {name} (accepted: {accepted})"
            ),
            Self::MissingArgument(name) => write!(formatter, "missing tool argument {name}"),
            Self::InvalidIdentifier(name) => {
                write!(formatter, "{name} must contain 1 to 256 characters")
            }
            Self::InvalidArgument(name) => write!(formatter, "invalid tool argument {name}"),
            Self::InvalidArgumentAt { path, reason } => {
                write!(formatter, "invalid tool argument {path}: {reason}")
            }
        }
    }
}

impl std::error::Error for ToolCallValidationError {}

/// The arguments of a function tool, which the model must send as an object.
pub fn object_arguments(arguments: &Value) -> Result<&Map<String, Value>, ToolCallValidationError> {
    arguments
        .as_object()
        .ok_or(ToolCallValidationError::ArgumentsMustBeObject)
}

/// Decodes a whole argument object into a typed call. An error names the
/// field it is about (`invalid tool argument steps[1].status: ...`), so the
/// model can correct that field alone. Argument types ignore unknown
/// fields and treat a missing or null optional field as absent; they never
/// use `deny_unknown_fields`.
pub fn decode_arguments<T: serde::de::DeserializeOwned>(
    arguments: &Map<String, Value>,
) -> Result<T, ToolCallValidationError> {
    serde_path_to_error::deserialize(Value::Object(arguments.clone())).map_err(|error| {
        let path = error.path().to_string();
        let reason = bounded_reason(error.into_inner().to_string());
        ToolCallValidationError::InvalidArgumentAt {
            // The root path is `.`: an error about the object itself, such
            // as a missing field, which serde names in the reason.
            path: if path == "." {
                "arguments".to_string()
            } else {
                path
            },
            reason,
        }
    })
}

/// serde's messages are short, but a value it quotes comes from the model:
/// keep the reason bounded on a UTF-8 boundary.
fn bounded_reason(mut reason: String) -> String {
    const MAXIMUM_BYTES: usize = 256;
    if reason.len() > MAXIMUM_BYTES {
        let mut end = MAXIMUM_BYTES;
        while !reason.is_char_boundary(end) {
            end -= 1;
        }
        reason.truncate(end);
        reason.push('…');
    }
    reason
}
