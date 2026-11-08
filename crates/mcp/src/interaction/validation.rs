//! Elicitation request admission and decision validation against the requested form.

use hanafubuki_agent_tool_contracts::{canonical_digest, validate_json_schema};
use rmcp::model::ElicitRequestParams;
use serde_json::Value;

use super::{
    McpCallContext, McpElicitationDecision, McpElicitationKind, McpElicitationRequest,
    McpElicitationRequestId,
};
use crate::{McpConnectionGeneration, McpError, McpServerId};

const MAX_ELICITATION_MESSAGE_BYTES: usize = 8 * 1024;

const MAX_ELICITATION_SCHEMA_BYTES: usize = 128 * 1024;

const MAX_ELICITATION_CONTENT_BYTES: usize = 128 * 1024;

pub(super) fn build_request(
    id: McpElicitationRequestId,
    server_id: McpServerId,
    connection_generation: McpConnectionGeneration,
    call_context: McpCallContext,
    params: ElicitRequestParams,
) -> Result<McpElicitationRequest, McpError> {
    let (message, kind) = match params {
        ElicitRequestParams::FormElicitationParams {
            message,
            requested_schema,
            ..
        } => {
            let schema =
                serde_json::to_value(requested_schema).map_err(|_| McpError::LimitExceeded)?;
            validate_json_schema(&schema).map_err(|_| McpError::LimitExceeded)?;
            let encoded = serde_json::to_vec(&schema).map_err(|_| McpError::LimitExceeded)?;
            if encoded.len() > MAX_ELICITATION_SCHEMA_BYTES {
                return Err(McpError::LimitExceeded);
            }
            let schema_digest = canonical_digest(&schema).map_err(|_| McpError::LimitExceeded)?;
            (
                message,
                McpElicitationKind::Form {
                    schema,
                    schema_digest,
                },
            )
        }
        ElicitRequestParams::UrlElicitationParams {
            message,
            url,
            elicitation_id,
            ..
        } => {
            let parsed = url::Url::parse(&url)
                .map_err(|_| McpError::InvalidConfig("elicitation URL is invalid".into()))?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(McpError::InvalidConfig(
                    "elicitation URL must be an HTTP(S) URL without credentials".into(),
                ));
            }
            if elicitation_id.is_empty() || elicitation_id.len() > 512 {
                return Err(McpError::LimitExceeded);
            }
            (
                message,
                McpElicitationKind::Url {
                    url,
                    remote_elicitation_id: elicitation_id,
                },
            )
        }
        _ => {
            return Err(McpError::InvalidConfig(
                "unsupported MCP elicitation mode".into(),
            ));
        }
    };
    if message.trim().is_empty() || message.len() > MAX_ELICITATION_MESSAGE_BYTES {
        return Err(McpError::LimitExceeded);
    }
    Ok(McpElicitationRequest {
        id,
        server_id,
        connection_generation,
        session_id: call_context.session_id,
        turn_id: call_context.turn_id,
        call_id: call_context.call_id,
        tool_generation: call_context.tool_generation,
        message,
        kind,
    })
}

pub(super) fn validate_decision(
    request: &McpElicitationRequest,
    decision: &McpElicitationDecision,
) -> Result<(), McpError> {
    match (decision, &request.kind) {
        (McpElicitationDecision::Accept { content }, McpElicitationKind::Form { schema, .. }) => {
            let bytes = serde_json::to_vec(content).map_err(|_| McpError::LimitExceeded)?;
            if bytes.len() > MAX_ELICITATION_CONTENT_BYTES || !content.is_object() {
                return Err(McpError::LimitExceeded);
            }
            validate_form_content(schema, content)
        }
        (McpElicitationDecision::Accept { content }, McpElicitationKind::Url { .. })
            if !content.is_null() =>
        {
            Err(McpError::InvalidConfig(
                "URL elicitation acceptance cannot include form content".into(),
            ))
        }
        _ => Ok(()),
    }
}

fn validate_form_content(schema: &Value, content: &Value) -> Result<(), McpError> {
    let object = content
        .as_object()
        .ok_or_else(|| McpError::InvalidConfig("elicitation content must be an object".into()))?;
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(field) {
                return Err(McpError::InvalidConfig(
                    "elicitation content does not satisfy the requested schema".into(),
                ));
            }
        }
    }
    let properties = schema.get("properties").and_then(Value::as_object);
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false)
        && object
            .keys()
            .any(|field| properties.is_none_or(|properties| !properties.contains_key(field)))
    {
        return Err(McpError::InvalidConfig(
            "elicitation content includes an unexpected field".into(),
        ));
    }
    if let Some(properties) = properties {
        for (field, value) in object {
            let Some(field_schema) = properties.get(field) else {
                continue;
            };
            if let Some(kind) = field_schema.get("type").and_then(Value::as_str) {
                let valid = match kind {
                    "string" => value.is_string(),
                    "number" => value.is_number(),
                    "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
                    "boolean" => value.is_boolean(),
                    _ => false,
                };
                if !valid {
                    return Err(McpError::InvalidConfig(
                        "elicitation content has a field with the wrong type".into(),
                    ));
                }
            }
            if let Some(allowed) = field_schema.get("enum").and_then(Value::as_array) {
                if !allowed.contains(value) {
                    return Err(McpError::InvalidConfig(
                        "elicitation content has a field outside the allowed choices".into(),
                    ));
                }
            }
            if let Some(text) = value.as_str() {
                if field_schema
                    .get("minLength")
                    .and_then(Value::as_u64)
                    .is_some_and(|minimum| text.chars().count() < minimum as usize)
                    || field_schema
                        .get("maxLength")
                        .and_then(Value::as_u64)
                        .is_some_and(|maximum| text.chars().count() > maximum as usize)
                {
                    return Err(McpError::InvalidConfig(
                        "elicitation content has a field outside the length bounds".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}
