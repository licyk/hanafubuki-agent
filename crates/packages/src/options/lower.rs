//! A `configSchema`, the JSON Schema a code package gives its settings,
//! as plugin options.
//!
//! Each property of the schema's `properties` becomes an option from its
//! `type`, `minimum` and `maximum`, `maxLength`, `enum`, `items` (with
//! `maxItems`), `default` and `description`, labelled by its `title` (else
//! its key), each choice by its value. `x-secret` makes a text an option
//! the forms do not show. A string or an `enum` with `null` among its
//! values is a text that may be unset. A keyword not among them constrains
//! nothing, as
//! in the kernel's check of a dynamic tool's arguments. A property that is
//! none of the kinds (an object, a list of anything but strings, a union of
//! types, an `enum` of anything but strings) is left out with an issue, and
//! its value reaches the guest unchecked.

use hanafubuki_agent_foundation::settings::{SettingChoice, SettingKind, SettingSpec};
use serde_json::{Map, Value};

use super::{PluginOptionSchema, MAX_PLUGIN_OPTIONS};

/// The options `schema` describes, in its properties' order, and what it
/// describes that cannot be one.
pub fn lower_config_schema(schema: &Map<String, Value>) -> (PluginOptionSchema, Vec<String>) {
    let mut lowered = PluginOptionSchema::default();
    let mut issues = Vec::new();
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return (lowered, issues);
    };
    for (key, property) in properties {
        if lowered.options.len() == MAX_PLUGIN_OPTIONS {
            issues.push(format!(
                "configSchema property `{key}` is left out: a plugin has at most \
                 {MAX_PLUGIN_OPTIONS} options"
            ));
            continue;
        }
        match lower_property(key, property) {
            Ok((option, issue)) => {
                lowered.options.push(option);
                issues.extend(issue);
            }
            Err(reason) => issues.push(format!(
                "configSchema property `{key}` {reason}, so it is not an option; its value \
                 reaches the plugin unchecked"
            )),
        }
    }
    (lowered, issues)
}

/// The option of one property, and an issue when its `default` does not
/// fit it.
fn lower_property(key: &str, property: &Value) -> Result<(SettingSpec, Option<String>), String> {
    let property = property
        .as_object()
        .ok_or_else(|| "is not a schema".to_string())?;
    let label = property
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(key);
    let mut option = SettingSpec {
        summary: property
            .get("description")
            .and_then(Value::as_str)
            .and_then(|text| text.lines().next())
            .unwrap_or_default()
            .to_string(),
        ..SettingSpec::new(key, label, kind(property)?)
    };
    let mut issue = None;
    if let Some(default) = property.get("default") {
        match option.check(default) {
            Ok(value) => option.default = Some(value),
            Err(reason) => {
                issue = Some(format!(
                    "configSchema property `{key}` has a default that does not fit it ({reason}); it has none"
                ))
            }
        }
    }
    Ok((option, issue))
}

fn kind(property: &Map<String, Value>) -> Result<SettingKind, String> {
    let (choices, nullable) = match property.get("enum") {
        Some(values) => {
            let (values, nullable) = strings(values)?;
            (Some(values), nullable)
        }
        None => (None, types_allow_null(property)?),
    };
    let kind = match property.get("type") {
        None if nullable => Some("string"),
        None => None,
        Some(Value::String(kind)) => Some(kind.as_str()),
        Some(Value::Array(types)) if nullable && names_text(types) => Some("string"),
        Some(_) => return Err("has more than one type".to_string()),
    };
    let (minimum, maximum) = (bound(property, "minimum"), bound(property, "maximum"));
    if minimum
        .zip(maximum)
        .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        return Err("has a minimum above its maximum".to_string());
    }
    match (kind, choices) {
        (None | Some("string"), Some(values)) if nullable => Ok(SettingKind::ChoiceOrText {
            values: values
                .into_iter()
                .map(|value| SettingChoice::new(value.clone(), value))
                .collect(),
            maximum_chars: count(property, "maxLength"),
            nullable,
        }),
        (None | Some("string"), Some(values)) => Ok(SettingKind::Choice {
            values: values
                .into_iter()
                .map(|value| SettingChoice::new(value.clone(), value))
                .collect(),
        }),
        (Some(_), Some(_)) => Err("has an enum of a type other than string".to_string()),
        (Some("boolean"), None) => Ok(SettingKind::Boolean),
        (Some("integer"), None) => Ok(SettingKind::Integer {
            minimum: minimum.map(|value| value.ceil() as i64),
            maximum: maximum.map(|value| value.floor() as i64),
        }),
        (Some("number"), None) => Ok(SettingKind::Number { minimum, maximum }),
        (Some("string"), None) => Ok(SettingKind::Text {
            maximum_chars: count(property, "maxLength"),
            secret: property
                .get("x-secret")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            nullable,
        }),
        (Some("array"), None) => {
            let items = property
                .get("items")
                .and_then(Value::as_object)
                .filter(|items| items.get("type").and_then(Value::as_str) == Some("string"))
                .ok_or_else(|| "is a list of something other than strings".to_string())?;
            Ok(SettingKind::TextList {
                maximum_items: count(property, "maxItems"),
                maximum_item_chars: count(items, "maxLength"),
            })
        }
        (Some(kind), None) => Err(format!("is of type `{kind}`")),
        (None, None) => Err("has no type".to_string()),
    }
}

/// An `enum`'s string values, and whether it also has `null`: a choice the
/// forms do not offer may still be unset, as a text is.
fn strings(values: &Value) -> Result<(Vec<String>, bool), String> {
    let values = values
        .as_array()
        .ok_or_else(|| "has an enum that is not a list".to_string())?;
    let (values, nulls) = values
        .iter()
        .partition::<Vec<_>, _>(|value| **value != Value::Null);
    let texts = values
        .iter()
        .map(|value| value.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()
        .filter(|texts| !texts.is_empty())
        .ok_or_else(|| "has an enum that is not a list of strings".to_string())?;
    // One `null` at most: a list of them offers nothing to choose.
    let nullable = nulls.len() == 1;
    if nulls.len() > 1 {
        return Err("has an enum that is not a list of strings".to_string());
    }
    Ok((texts, nullable))
}

/// Whether a union of types names `null`, as a text that may be unset
/// does.
fn types_allow_null(property: &Map<String, Value>) -> Result<bool, String> {
    let Some(Value::Array(types)) = property.get("type") else {
        return Ok(false);
    };
    Ok(names(types, "null"))
}

/// Whether a union of types is `string` and `null` alone.
fn names_text(types: &[Value]) -> bool {
    names(types, "string")
        && types
            .iter()
            .all(|kind| matches!(kind.as_str(), Some("string" | "null")))
}

fn names(types: &[Value], kind: &str) -> bool {
    types.iter().any(|value| value.as_str() == Some(kind))
}

fn bound(property: &Map<String, Value>, keyword: &str) -> Option<f64> {
    property.get(keyword)?.as_f64()
}

/// A length bound; unbounded without one, or past what a `u32` counts.
fn count(property: &Map<String, Value>, keyword: &str) -> Option<u32> {
    let value = property.get(keyword)?.as_u64()?;
    u32::try_from(value).ok()
}
