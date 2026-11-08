//! Field-by-field reading of YAML that third parties author.
//!
//! `SKILL.md` frontmatter and `agents/openai.yaml` are written by people and
//! by other tools, so a key this catalog knows may carry a type it does not
//! expect (`version: 1.0`, `display_name: 42`). Decoding the whole document
//! into one typed struct turns that one key into a failure of the file. These
//! helpers read a parsed [`Value`] one known key at a time instead: a value of
//! the wrong type is dropped and described in `issues`, and every other key
//! keeps its value. A `null` reads as absent, as an omitted key does.

use serde_yaml::{Mapping, Value};

/// The value of the first of `keys` present and not `null`.
pub(crate) fn field<'a>(mapping: &'a Mapping, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| mapping.get(*key))
        .find(|value| !value.is_null())
}

/// A string under one of `keys`. Another type is ignored with an issue that
/// names the field as `label`.
pub(crate) fn optional_string(
    mapping: &Mapping,
    keys: &[&str],
    label: &str,
    issues: &mut Vec<String>,
) -> Option<String> {
    match field(mapping, keys)? {
        Value::String(value) => Some(value.clone()),
        other => {
            issues.push(ignored(label, "a string", other));
            None
        }
    }
}

pub(crate) fn optional_bool(
    mapping: &Mapping,
    key: &str,
    label: &str,
    issues: &mut Vec<String>,
) -> Option<bool> {
    match field(mapping, &[key])? {
        Value::Bool(value) => Some(*value),
        other => {
            issues.push(ignored(label, "true or false", other));
            None
        }
    }
}

pub(crate) fn optional_mapping<'a>(
    mapping: &'a Mapping,
    key: &str,
    label: &str,
    issues: &mut Vec<String>,
) -> Option<&'a Mapping> {
    match field(mapping, &[key])? {
        Value::Mapping(value) => Some(value),
        other => {
            issues.push(ignored(label, "a mapping", other));
            None
        }
    }
}

pub(crate) fn optional_sequence<'a>(
    mapping: &'a Mapping,
    key: &str,
    label: &str,
    issues: &mut Vec<String>,
) -> Option<&'a [Value]> {
    match field(mapping, &[key])? {
        Value::Sequence(value) => Some(value),
        other => {
            issues.push(ignored(label, "a list", other));
            None
        }
    }
}

/// The issue for a value of the wrong type that was dropped.
pub(crate) fn ignored(label: &str, expected: &str, found: &Value) -> String {
    format!(
        "ignored {label}: expected {expected}, found {}",
        kind(found)
    )
}

pub(crate) fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Sequence(_) => "a list",
        Value::Mapping(_) => "a mapping",
        Value::Tagged(_) => "a tagged value",
    }
}
