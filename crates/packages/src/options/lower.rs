//! A `configSchema`, the JSON Schema a code package gives its settings,
//! as plugin options.
//!
//! Each property of the schema's `properties` becomes an option from its
//! `type`, `minimum` and `maximum`, `maxLength`, `enum`, `items` (with
//! `maxItems`), `default` and `description`. A keyword not among them
//! constrains nothing, as in the kernel's check of a dynamic tool's
//! arguments. A property that is none of the kinds (an object, a list of
//! anything but strings, a union of types) is left out with an issue, and
//! its value reaches the guest unchecked.

use serde_json::{Map, Value};

use super::{PluginOption, PluginOptionKind, PluginOptionSchema, MAX_PLUGIN_OPTIONS};

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
fn lower_property(key: &str, property: &Value) -> Result<(PluginOption, Option<String>), String> {
    let property = property
        .as_object()
        .ok_or_else(|| "is not a schema".to_string())?;
    let mut option = PluginOption {
        key: key.to_string(),
        summary: property
            .get("description")
            .and_then(Value::as_str)
            .and_then(|text| text.lines().next())
            .unwrap_or_default()
            .to_string(),
        kind: kind(property)?,
        default: None,
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

fn kind(property: &Map<String, Value>) -> Result<PluginOptionKind, String> {
    let choices = property.get("enum").map(strings).transpose()?;
    let kind = match property.get("type") {
        None => None,
        Some(Value::String(kind)) => Some(kind.as_str()),
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
        (None | Some("string"), Some(values)) => Ok(PluginOptionKind::Choice { values }),
        (Some(_), Some(_)) => Err("has an enum of a type other than string".to_string()),
        (Some("boolean"), None) => Ok(PluginOptionKind::Boolean),
        (Some("integer"), None) => Ok(PluginOptionKind::Integer {
            minimum: minimum.map_or(i64::MIN, |value| value.ceil() as i64),
            maximum: maximum.map_or(i64::MAX, |value| value.floor() as i64),
        }),
        (Some("number"), None) => Ok(PluginOptionKind::Number {
            minimum: minimum.unwrap_or(f64::MIN),
            maximum: maximum.unwrap_or(f64::MAX),
        }),
        (Some("string"), None) => Ok(PluginOptionKind::Text {
            maximum_chars: count(property, "maxLength"),
            secret: false,
        }),
        (Some("array"), None) => {
            let items = property
                .get("items")
                .and_then(Value::as_object)
                .filter(|items| items.get("type").and_then(Value::as_str) == Some("string"))
                .ok_or_else(|| "is a list of something other than strings".to_string())?;
            Ok(PluginOptionKind::TextList {
                maximum_items: count(property, "maxItems"),
                maximum_item_chars: count(items, "maxLength"),
            })
        }
        (Some(kind), None) => Err(format!("is of type `{kind}`")),
        (None, None) => Err("has no type".to_string()),
    }
}

/// An `enum`'s values, when they are all strings.
fn strings(values: &Value) -> Result<Vec<String>, String> {
    values
        .as_array()
        .filter(|values| !values.is_empty())
        .and_then(|values| {
            values
                .iter()
                .map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .ok_or_else(|| "has an enum that is not a list of strings".to_string())
}

fn bound(property: &Map<String, Value>, keyword: &str) -> Option<f64> {
    property.get(keyword)?.as_f64()
}

/// A length bound; unbounded without one.
fn count(property: &Map<String, Value>, keyword: &str) -> usize {
    property
        .get(keyword)
        .and_then(Value::as_u64)
        .map_or(usize::MAX, |value| {
            usize::try_from(value).unwrap_or(usize::MAX)
        })
}
