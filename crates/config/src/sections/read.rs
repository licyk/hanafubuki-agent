//! Reading a layer's settings by their specs, and checking a typed value
//! against them.

use std::collections::BTreeSet;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use super::specs;
use crate::fields::Fields;

/// What reading one layer found besides the values.
#[derive(Debug, Default)]
pub(crate) struct LayerRead {
    /// The values that could not be used, each naming its key.
    pub(crate) repairs: Vec<String>,
    /// The keys whose value this layer sets and the settings use.
    pub(crate) set: BTreeSet<String>,
}

/// One layer's document, read setting by setting.
pub(crate) struct SettingsRead<'a> {
    fields: Fields<'a>,
    set: &'a mut BTreeSet<String>,
    /// The tables on a key's way that are not tables, reported once.
    reported: BTreeSet<String>,
}

impl<'a> SettingsRead<'a> {
    pub(crate) fn new(document: &'a Map<String, Value>, read: &'a mut LayerRead) -> Self {
        Self {
            fields: Fields::new(document, &mut read.repairs),
            set: &mut read.set,
            reported: BTreeSet::new(),
        }
    }

    /// The fields of the document, for the tables read by hand.
    pub(crate) fn fields(&mut self) -> &mut Fields<'a> {
        &mut self.fields
    }

    /// Records that the layer's value of `key` is used.
    pub(crate) fn mark(&mut self, key: &str) {
        self.set.insert(key.to_string());
    }

    /// Records that `key` fell back to the layer below, and is not set here.
    pub(crate) fn unmark(&mut self, key: &str) {
        self.set.remove(key);
    }

    /// The value at `key` (a setting, or a struct of the settings under it)
    /// read over `below`. Each setting the document holds replaces the one
    /// below when its spec reads it and the result passes `rule`; any other
    /// is a repair and keeps the value below. A stored alias or disabled
    /// choice is read as it is: only a write refuses them.
    pub(crate) fn read<T>(
        &mut self,
        key: &str,
        below: &T,
        rule: impl Fn(&T) -> Result<(), String>,
    ) -> T
    where
        T: Serialize + DeserializeOwned + Clone,
    {
        let mut value = below.clone();
        let Ok(mut current) = serde_json::to_value(below) else {
            return value;
        };
        for spec in specs().iter().filter(|spec| is_under(&spec.key, key)) {
            let Some(raw) = self.raw(&spec.key) else {
                continue;
            };
            let relative = spec.key[key.len()..].trim_start_matches('.');
            let accepted = spec.read(raw).ok().and_then(|read| {
                let mut candidate = current.clone();
                set_at(&mut candidate, relative, read.to_json());
                let typed = serde_json::from_value::<T>(candidate.clone()).ok()?;
                rule(&typed).is_ok().then_some((candidate, typed))
            });
            match accepted {
                Some((candidate, typed)) => {
                    current = candidate;
                    value = typed;
                    self.mark(&spec.key);
                }
                None => self.fields.invalid(&spec.key, None),
            }
        }
        value
    }

    /// The value at the dotted `key`, when the document holds one; a table
    /// on its way that is not one is a repair, reported once.
    fn raw(&mut self, key: &str) -> Option<&'a Value> {
        let mut table = self.fields.object();
        let mut path = String::new();
        let mut segments = key.split('.').peekable();
        while let Some(segment) = segments.next() {
            if !path.is_empty() {
                path.push('.');
            }
            path.push_str(segment);
            let value = table.get(segment)?;
            if segments.peek().is_none() {
                return Some(value);
            }
            match value.as_object() {
                Some(next) => table = next,
                None => {
                    if self.reported.insert(path.clone()) {
                        self.fields.invalid(&path, None);
                    }
                    return None;
                }
            }
        }
        None
    }
}

/// No rule beyond the specs, for [`SettingsRead::read`].
pub(crate) fn no_rule<T>(_: &T) -> Result<(), String> {
    Ok(())
}

/// Checks each setting under `key` that `value` (the value at `key`) sets
/// against its spec, strictly, as a write is checked. An unset value
/// (`None`) is not checked.
pub(crate) fn check_specs<T: Serialize>(key: &str, value: &T) -> Result<(), String> {
    let value = serde_json::to_value(value)
        .map_err(|_| format!("`{key}` cannot be encoded as settings"))?;
    for spec in specs().iter().filter(|spec| is_under(&spec.key, key)) {
        let relative = spec.key[key.len()..].trim_start_matches('.');
        let leaf = relative
            .split('.')
            .filter(|segment| !segment.is_empty())
            .try_fold(&value, |value, segment| value.get(segment));
        if let Some(leaf) = leaf.filter(|leaf| !leaf.is_null()) {
            spec.check(leaf)?;
        }
    }
    Ok(())
}

/// Whether the setting `spec_key` is `key` or under it.
fn is_under(spec_key: &str, key: &str) -> bool {
    spec_key
        .strip_prefix(key)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// Sets the dotted `relative` path of `value` (all of it when empty).
fn set_at(value: &mut Value, relative: &str, new: Value) {
    if relative.is_empty() {
        *value = new;
        return;
    }
    let mut target = value;
    for segment in relative.split('.') {
        let Value::Object(table) = target else {
            return;
        };
        target = table.entry(segment).or_insert(Value::Null);
    }
    *target = new;
}
