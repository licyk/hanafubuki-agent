//! A record's fields, read from and written into its serialized form by
//! their specs: the collections whose records are typed structs (a
//! Provider, the model policy, an MCP server) share these, so a field the
//! specs do not name (per-model overrides) is kept as it is. A map of the
//! record (an MCP server's environment, its per-tool policies) is a list of
//! `NAME=VALUE` texts while it is read and written ([`maps_as_lists`],
//! [`lists_as_maps`]).

use std::collections::BTreeMap;

use hanafubuki_agent_config::{SettingKind, SettingSpec};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use super::SettingsRecord;

/// The record `id` of `fields`, its values read from `record` (its
/// serialized form); a secret field says only whether it is set.
pub(super) fn record_of<T: Serialize>(
    id: &str,
    label: &str,
    record: &T,
    fields: &[SettingSpec],
) -> SettingsRecord {
    let record = serde_json::to_value(record).unwrap_or(Value::Null);
    let mut values = BTreeMap::new();
    let mut secrets_set = Vec::new();
    for spec in fields {
        let Some(value) = at(&record, &spec.key).filter(|value| !value.is_null()) else {
            continue;
        };
        if is_secret(spec) {
            secrets_set.push(spec.key.clone());
        } else if let Ok(read) = spec.read(value) {
            values.insert(spec.key.clone(), read);
        }
    }
    SettingsRecord {
        id: id.to_string(),
        label: label.to_string(),
        summary: String::new(),
        state: None,
        issue: None,
        read_only: None,
        values,
        secrets_set,
    }
}

/// `fields` with their defaults the values of `template`, the record an
/// `add` writes its values over: an add form starts from what it would
/// write.
pub(super) fn with_defaults<T: Serialize>(
    mut fields: Vec<SettingSpec>,
    template: &T,
) -> Vec<SettingSpec> {
    let mut defaults = record_of("", "", template, &fields).values;
    for spec in &mut fields {
        spec.default = defaults.remove(&spec.key);
    }
    fields
}

/// `record` with `values` written over it, each checked against its field's
/// spec, read back as `R`. `null` unsets an optional field, which `R`
/// refuses for one that is not.
pub(super) fn merged<R: DeserializeOwned>(
    record: &impl Serialize,
    fields: &[SettingSpec],
    values: &Map<String, Value>,
) -> Result<R, String> {
    let mut record =
        serde_json::to_value(record).map_err(|_| "the record cannot be encoded".to_string())?;
    for (key, raw) in values {
        let spec = fields
            .iter()
            .find(|spec| spec.key == *key)
            .ok_or_else(|| format!("`{key}` is not a field of this record"))?;
        let value = match raw {
            Value::Null => Value::Null,
            raw => spec.check(raw)?.to_json(),
        };
        set_at(&mut record, key, value);
    }
    serde_json::from_value(record).map_err(|error| format!("the record cannot be written: {error}"))
}

fn is_secret(spec: &SettingSpec) -> bool {
    matches!(spec.kind, SettingKind::Text { secret: true, .. })
}

/// The value at the dotted `key` of `value`.
fn at<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    key.split('.')
        .try_fold(value, |value, segment| value.get(segment))
}

/// Sets the dotted `key` of `value`, making the objects on the way.
fn set_at(value: &mut Value, key: &str, new: Value) {
    let mut target = value;
    for segment in key.split('.') {
        if !target.is_object() {
            *target = Value::Object(Map::new());
        }
        let Value::Object(object) = target else {
            return;
        };
        target = object.entry(segment).or_insert(Value::Null);
    }
    *target = new;
}

/// How a map's values are written from a `NAME=VALUE` text.
#[derive(Clone, Copy)]
pub(super) enum PairValue {
    Text,
    /// A whole number.
    Integer,
}

/// `record` with the maps at `keys` as lists of `NAME=VALUE` texts, which
/// a `TextList` field reads.
pub(super) fn maps_as_lists(record: &mut Value, keys: &[(&str, PairValue)]) {
    for (key, _) in keys {
        let Some(Value::Object(map)) = at(record, key) else {
            continue;
        };
        let items = map
            .iter()
            .map(|(name, value)| match value {
                Value::String(text) => Value::String(format!("{name}={text}")),
                other => Value::String(format!("{name}={other}")),
            })
            .collect();
        set_at(record, key, Value::Array(items));
    }
}

/// The lists of `NAME=VALUE` texts at `keys` back into maps; an item that is
/// not one, or a name given twice, is refused.
pub(super) fn lists_as_maps(record: &mut Value, keys: &[(&str, PairValue)]) -> Result<(), String> {
    for (key, kind) in keys {
        let Some(Value::Array(items)) = at(record, key) else {
            continue;
        };
        let mut map = Map::new();
        for item in items {
            let text = item.as_str().unwrap_or_default();
            let (name, value) = text
                .split_once('=')
                .map(|(name, value)| (name.trim(), value.trim()))
                .filter(|(name, _)| !name.is_empty())
                .ok_or_else(|| format!("`{text}` is not NAME=VALUE"))?;
            let value = match kind {
                PairValue::Text => Value::String(value.to_string()),
                PairValue::Integer => value
                    .parse::<u64>()
                    .map(Value::from)
                    .map_err(|_| format!("`{text}`: the value is not a whole number"))?,
            };
            if map.insert(name.to_string(), value).is_some() {
                return Err(format!("`{name}` is given twice"));
            }
        }
        set_at(record, key, Value::Object(map));
    }
    Ok(())
}
