use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::ToolContractError;

pub const MAX_TOOL_DESCRIPTION_BYTES: usize = 16 * 1024;
pub const MAX_TOOL_SCHEMA_BYTES: usize = 256 * 1024;
pub const MAX_TOOL_SCHEMA_DEPTH: usize = 32;
pub const MAX_FREEFORM_GRAMMAR_BYTES: usize = 128 * 1024;

pub fn validate_description(description: &str) -> Result<(), ToolContractError> {
    if description.len() > MAX_TOOL_DESCRIPTION_BYTES {
        return Err(ToolContractError::DescriptionTooLong);
    }
    Ok(())
}

pub fn validate_json_schema(schema: &Value) -> Result<(), ToolContractError> {
    let bytes = serde_json::to_vec(schema).map_err(|_| ToolContractError::SchemaTooLarge)?;
    if bytes.len() > MAX_TOOL_SCHEMA_BYTES {
        return Err(ToolContractError::SchemaTooLarge);
    }
    if json_depth(schema, 0) > MAX_TOOL_SCHEMA_DEPTH {
        return Err(ToolContractError::SchemaTooDeep);
    }
    if !schema.is_object() && !schema.is_boolean() {
        return Err(ToolContractError::SchemaTooLarge);
    }
    Ok(())
}

pub fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut canonical = Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_json(&object[key]));
            }
            Value::Object(canonical)
        }
        primitive => primitive.clone(),
    }
}

pub fn canonical_digest<T: serde::Serialize>(value: &T) -> Result<[u8; 32], ToolContractError> {
    let value = serde_json::to_value(value).map_err(|_| ToolContractError::SchemaTooLarge)?;
    let bytes = serde_json::to_vec(&canonical_json(&value))
        .map_err(|_| ToolContractError::SchemaTooLarge)?;
    Ok(Sha256::digest(bytes).into())
}

fn json_depth(value: &Value, depth: usize) -> usize {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| json_depth(item, depth + 1))
            .max()
            .unwrap_or(depth + 1),
        Value::Object(object) => object
            .values()
            .map(|item| json_depth(item, depth + 1))
            .max()
            .unwrap_or(depth + 1),
        _ => depth,
    }
}
