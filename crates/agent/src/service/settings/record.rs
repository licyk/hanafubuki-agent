//! A record's fields, read from and written into its serialized form by
//! their specs: the collections whose records are typed structs (a
//! Provider, the model policy, an MCP server) share these, so a field the
//! specs do not name (request headers, per-tool maps, per-model overrides)
//! is kept as it is.

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
        values,
        secrets_set,
    }
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
