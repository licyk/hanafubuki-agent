use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentToolCatalogError {
    Cancelled,
    InvalidToolName,
    InvalidSchema,
    DuplicateToolId,
    TooManyTools { limit: usize },
    SchemaBytesExceeded { limit: usize },
    ToolNotAdvertised(String),
    ProviderFailed(String),
    InvalidArguments(String),
    InvalidRuntimeBinding,
}

impl std::fmt::Display for AgentToolCatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("tool catalog construction was cancelled"),
            Self::InvalidToolName => {
                formatter.write_str("tool catalog contains an invalid tool name")
            }
            Self::InvalidSchema => formatter.write_str("tool catalog contains an invalid schema"),
            Self::DuplicateToolId => {
                formatter.write_str("tool catalog contains a duplicate provider/tool id")
            }
            Self::TooManyTools { limit } => {
                write!(formatter, "tool catalog exceeds {limit} entries")
            }
            Self::SchemaBytesExceeded { limit } => {
                write!(formatter, "tool catalog schemas exceed {limit} bytes")
            }
            Self::ToolNotAdvertised(name) => write!(
                formatter,
                "tool {name} was not advertised for this sampling step"
            ),
            Self::ProviderFailed(message) => {
                write!(formatter, "tool catalog provider failed: {message}")
            }
            Self::InvalidArguments(message) => {
                write!(formatter, "dynamic tool arguments are invalid: {message}")
            }
            Self::InvalidRuntimeBinding => {
                formatter.write_str("planned handler tool has no handler binding")
            }
        }
    }
}

/// Checks a call's arguments against the schema of the tool it calls. The
/// schema is the plan's, validated once when its tool was planned
/// (`validate_planned_tool`), so it is not validated again per call.
pub fn validate_dynamic_tool_arguments(
    schema: &Value,
    arguments: &Value,
) -> Result<(), AgentToolCatalogError> {
    let bytes = serde_json::to_vec(arguments)
        .map_err(|_| AgentToolCatalogError::InvalidArguments("invalid JSON".to_string()))?;
    if bytes.len() > hanafubuki_agent_tool_contracts::MAX_TOOL_SCHEMA_BYTES {
        return Err(AgentToolCatalogError::InvalidArguments(
            "payload exceeds the byte limit".to_string(),
        ));
    }
    validate_schema_value(schema, arguments, 0, "arguments")
}

pub(super) fn validate_schema_value(
    schema: &Value,
    value: &Value,
    depth: usize,
    path: &str,
) -> Result<(), AgentToolCatalogError> {
    if depth > hanafubuki_agent_tool_contracts::MAX_TOOL_SCHEMA_DEPTH {
        return Err(AgentToolCatalogError::InvalidArguments(
            "payload exceeds the depth limit".to_string(),
        ));
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if !values.contains(value) {
            return Err(invalid_argument(path, "is not an allowed value"));
        }
    }
    if let Some(types) = schema.get("type") {
        let matches = match types {
            Value::String(kind) => value_matches_type(value, kind),
            Value::Array(kinds) => kinds
                .iter()
                .filter_map(Value::as_str)
                .any(|kind| value_matches_type(value, kind)),
            _ => false,
        };
        if !matches {
            return Err(invalid_argument(path, "has the wrong JSON type"));
        }
    }
    if let Some(object) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for field in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(field) {
                    return Err(invalid_argument(&format!("{path}.{field}"), "is required"));
                }
            }
        }
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            for field in object.keys() {
                if !properties.is_some_and(|properties| properties.contains_key(field)) {
                    return Err(invalid_argument(
                        &format!("{path}.{field}"),
                        "is not a field this tool takes",
                    ));
                }
            }
        }
        if let Some(properties) = properties {
            for (field, field_schema) in properties {
                if let Some(field_value) = object.get(field) {
                    validate_schema_value(
                        field_schema,
                        field_value,
                        depth + 1,
                        &format!("{path}.{field}"),
                    )?;
                }
            }
        }
    }
    if let Some(items) = value.as_array() {
        if let Some(limit) = schema.get("maxItems").and_then(Value::as_u64) {
            if items.len() as u64 > limit {
                return Err(invalid_argument(path, "has too many items"));
            }
        }
        if let Some(item_schema) = schema.get("items") {
            for (index, item) in items.iter().enumerate() {
                validate_schema_value(item_schema, item, depth + 1, &format!("{path}[{index}]"))?;
            }
        }
    }
    if let Some(text) = value.as_str() {
        if let Some(limit) = schema.get("maxLength").and_then(Value::as_u64) {
            if text.chars().count() as u64 > limit {
                return Err(invalid_argument(path, "exceeds the string length limit"));
            }
        }
    }
    Ok(())
}

/// A sourced schema comes from outside the application: a `type` this
/// check does not know constrains nothing here (the server validates its own
/// arguments), so it matches any value instead of rejecting every call. An
/// integral number such as `1.0` is an integer.
pub(super) fn value_matches_type(value: &Value, kind: &str) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "integer" => {
            value.as_i64().is_some()
                || value.as_u64().is_some()
                || value.as_f64().is_some_and(|value| value.fract() == 0.0)
        }
        "string" => value.is_string(),
        _ => true,
    }
}

pub(super) fn invalid_argument(path: &str, message: &str) -> AgentToolCatalogError {
    AgentToolCatalogError::InvalidArguments(format!("{path} {message}"))
}

impl std::error::Error for AgentToolCatalogError {}
