//! Field-by-field reading of one persisted object.
//!
//! Every field decodes on its own: a missing field takes its default, an
//! invalid one takes its default and is recorded as a repair, and every other
//! field keeps what the user configured.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// What one field held.
pub(crate) enum FieldRead<T> {
    Valid(T),
    Missing,
    Invalid,
}

pub(crate) struct Fields<'a> {
    object: &'a Map<String, Value>,
    path: String,
    repairs: &'a mut Vec<String>,
}

impl<'a> Fields<'a> {
    pub(crate) fn new(object: &'a Map<String, Value>, repairs: &'a mut Vec<String>) -> Self {
        Self::nested(object, String::new(), repairs)
    }

    /// A reader for an object nested at `path`, reporting under it.
    pub(crate) fn nested(
        object: &'a Map<String, Value>,
        path: String,
        repairs: &'a mut Vec<String>,
    ) -> Self {
        Self {
            object,
            path,
            repairs,
        }
    }

    pub(crate) fn path(&self, field: &str) -> String {
        if self.path.is_empty() {
            field.to_string()
        } else {
            format!("{}.{field}", self.path)
        }
    }

    pub(crate) fn raw(&self, field: &str) -> Option<&'a Value> {
        self.object.get(field)
    }

    /// Decodes and checks a field without recording anything.
    pub(crate) fn read_quiet<T: DeserializeOwned>(
        &self,
        field: &str,
        check: impl FnOnce(T) -> Result<T, String>,
    ) -> FieldRead<T> {
        let Some(value) = self.object.get(field) else {
            return FieldRead::Missing;
        };
        match serde_json::from_value::<T>(value.clone())
            .map_err(|_| ())
            .and_then(|value| check(value).map_err(|_| ()))
        {
            Ok(value) => FieldRead::Valid(value),
            Err(()) => FieldRead::Invalid,
        }
    }

    /// A field whose type is its whole contract.
    pub(crate) fn read<T: DeserializeOwned>(&mut self, field: &str, default: T) -> T {
        self.read_checked(field, default, Ok)
    }

    /// A field that must also pass `check`, which may normalize it.
    pub(crate) fn read_checked<T: DeserializeOwned>(
        &mut self,
        field: &str,
        default: T,
        check: impl FnOnce(T) -> Result<T, String>,
    ) -> T {
        match self.read_quiet(field, check) {
            FieldRead::Valid(value) => value,
            FieldRead::Missing => default,
            FieldRead::Invalid => {
                self.invalid(field, None);
                default
            }
        }
    }

    /// Records that `field` fell back to its default. `message` replaces the
    /// generic one when the fallback has a consequence the user should read.
    pub(crate) fn invalid(&mut self, field: &str, message: Option<&str>) {
        let path = self.path(field);
        self.repairs.push(format!(
            "{path}: {}",
            message.unwrap_or("the value is invalid and was replaced with its default")
        ));
    }

    pub(crate) fn repairs(&mut self) -> &mut Vec<String> {
        self.repairs
    }
}

/// Reads the object at `field` field by field with `decode`, starting from
/// `default`; a field that is not an object takes `default` whole.
pub(crate) fn read_object<T: Clone>(
    fields: &mut Fields<'_>,
    field: &str,
    default: &T,
    decode: impl FnOnce(&mut Fields<'_>, &mut T),
) -> T {
    let Some(raw) = fields.raw(field) else {
        return default.clone();
    };
    let Some(object) = raw.as_object() else {
        fields.invalid(field, None);
        return default.clone();
    };
    let mut value = default.clone();
    let path = fields.path(field);
    decode(
        &mut Fields::nested(object, path, fields.repairs()),
        &mut value,
    );
    value
}

/// Reads `field` into the part of `value` that `target` selects, keeping the
/// candidate only when the whole value still passes `validate`.
pub(crate) fn read_member<S: Clone, T: DeserializeOwned>(
    fields: &mut Fields<'_>,
    value: &mut S,
    field: &str,
    target: fn(&mut S) -> &mut T,
    validate: fn(&S) -> Result<(), String>,
) {
    match fields.read_quiet::<T>(field, Ok) {
        FieldRead::Valid(member) => {
            let mut candidate = value.clone();
            *target(&mut candidate) = member;
            if validate(&candidate).is_ok() {
                *value = candidate;
            } else {
                fields.invalid(field, None);
            }
        }
        FieldRead::Missing => {}
        FieldRead::Invalid => fields.invalid(field, None),
    }
}

/// The dotted paths of every key `raw` holds that `known` does not: keys a
/// newer version or the user wrote, which the Agent keeps on disk and
/// ignores. Lists are not descended.
pub(crate) fn unknown_keys(raw: &Map<String, Value>, known: &Map<String, Value>) -> Vec<String> {
    let mut unknown = Vec::new();
    collect_unknown(raw, known, "", &mut unknown);
    unknown
}

fn collect_unknown(
    raw: &Map<String, Value>,
    known: &Map<String, Value>,
    path: &str,
    unknown: &mut Vec<String>,
) {
    for (key, value) in raw {
        let child = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        match (value, known.get(key)) {
            (_, None) => unknown.push(child),
            (Value::Object(raw), Some(Value::Object(known))) => {
                collect_unknown(raw, known, &child, unknown)
            }
            _ => {}
        }
    }
}
