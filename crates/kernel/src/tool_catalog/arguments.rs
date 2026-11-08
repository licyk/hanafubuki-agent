//! The kernel's one tolerant pass over a call's arguments, guided by the
//! tool's own input schema, before mode policy, permission and the call:
//! - `1.0` becomes `1` and a numeric string becomes a number where the
//!   schema says `integer` or `number` (and not also `string`);
//! - a `null` for a property the schema neither requires nor declares
//!   nullable is removed, so it reads as absent;
//! - a key the schema does not declare is removed where the schema says
//!   `additionalProperties: false` (the tool would only reject it).
//!
//! Nothing else changes: any other unknown key stays for the handler to
//! ignore (a sourced tool's server may accept it), and no bound is applied
//! here, so
//! the pass never widens what a handler or the approval sees. Approval,
//! permission and the call all receive the normalized arguments; history
//! keeps what the model sent.

use serde_json::{Map, Number, Value};

pub(crate) fn normalize_tool_arguments(schema: &Value, arguments: &Value) -> Value {
    normalize(schema, arguments.clone(), 0)
}

fn normalize(schema: &Value, value: Value, depth: usize) -> Value {
    if depth > hanafubuki_agent_tool_contracts::MAX_TOOL_SCHEMA_DEPTH {
        return value;
    }
    let types = schema_types(schema);
    match value {
        Value::Number(number) => Value::Number(coerce_number(&types, number)),
        Value::String(text) => match coerce_numeric_text(&types, &text) {
            Some(number) => Value::Number(number),
            None => Value::String(text),
        },
        Value::Object(object) => Value::Object(normalize_object(schema, object, depth)),
        Value::Array(items) => match schema.get("items") {
            Some(item_schema) if item_schema.is_object() => Value::Array(
                items
                    .into_iter()
                    .map(|item| normalize(item_schema, item, depth + 1))
                    .collect(),
            ),
            _ => Value::Array(items),
        },
        other => other,
    }
}

fn normalize_object(
    schema: &Value,
    object: Map<String, Value>,
    depth: usize,
) -> Map<String, Value> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return object;
    };
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|required| {
            required
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    object
        .into_iter()
        .filter_map(|(key, value)| match properties.get(&key) {
            Some(property) => {
                if value.is_null()
                    && !required.contains(&key.as_str())
                    && !schema_types(property).contains(&"null")
                {
                    None
                } else {
                    Some((key, normalize(property, value, depth + 1)))
                }
            }
            None if closed => None,
            None => Some((key, value)),
        })
        .collect()
}

fn schema_types(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(kind)) => vec![kind.as_str()],
        Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// `1.0` where only an integer is accepted is that integer.
fn coerce_number(types: &[&str], number: Number) -> Number {
    if !types.contains(&"integer") || types.contains(&"number") {
        return number;
    }
    if number.is_i64() || number.is_u64() {
        return number;
    }
    number.as_f64().and_then(integral_number).unwrap_or(number)
}

/// A string where only a number is accepted, holding one, is that number.
fn coerce_numeric_text(types: &[&str], text: &str) -> Option<Number> {
    if types.contains(&"string") {
        return None;
    }
    let text = text.trim();
    if types.contains(&"integer") && !types.contains(&"number") {
        if let Ok(value) = text.parse::<i64>() {
            return Some(value.into());
        }
        if let Ok(value) = text.parse::<u64>() {
            return Some(value.into());
        }
        return text
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .and_then(integral_number);
    }
    if types.contains(&"number") {
        if let Ok(value) = text.parse::<i64>() {
            return Some(value.into());
        }
        return text
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .and_then(Number::from_f64);
    }
    None
}

fn integral_number(value: f64) -> Option<Number> {
    if value.fract() != 0.0 || !value.is_finite() {
        return None;
    }
    // Inside ±2^53 every integer is exact, so the conversion is lossless.
    const EXACT: f64 = 9_007_199_254_740_992.0;
    if value.abs() > EXACT {
        return None;
    }
    if value < 0.0 {
        Some((value as i64).into())
    } else {
        Some((value as u64).into())
    }
}

#[cfg(test)]
mod tests;
