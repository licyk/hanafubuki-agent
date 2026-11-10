//! Field-by-field reading of a JSON object a third party wrote: a value of
//! the wrong type is dropped with an issue naming it, and every other field
//! keeps its value. A `null` reads as absent, as an omitted key does.

use serde_json::{Map, Value};

pub(crate) struct Fields<'a> {
    object: &'a Map<String, Value>,
    prefix: String,
    issues: &'a mut Vec<String>,
}

impl<'a> Fields<'a> {
    pub(crate) fn new(
        object: &'a Map<String, Value>,
        prefix: &str,
        issues: &'a mut Vec<String>,
    ) -> Self {
        Self {
            object,
            prefix: prefix.to_string(),
            issues,
        }
    }

    pub(crate) fn label(&self, key: &str) -> String {
        format!("{}{key}", self.prefix)
    }

    pub(crate) fn issue(&mut self, key: &str, message: &str) {
        let label = self.label(key);
        self.issues.push(format!("ignoring {label}: {message}"));
    }

    pub(crate) fn issues(&mut self) -> &mut Vec<String> {
        self.issues
    }

    pub(crate) fn raw(&self, key: &str) -> Option<&'a Value> {
        self.object.get(key).filter(|value| !value.is_null())
    }

    pub(crate) fn string(&mut self, key: &str) -> Option<String> {
        match self.raw(key)? {
            Value::String(value) => Some(value.clone()),
            other => {
                self.issue(key, &format!("expected a string, found {}", kind(other)));
                None
            }
        }
    }

    pub(crate) fn bool(&mut self, key: &str) -> Option<bool> {
        match self.raw(key)? {
            Value::Bool(value) => Some(*value),
            other => {
                self.issue(
                    key,
                    &format!("expected true or false, found {}", kind(other)),
                );
                None
            }
        }
    }

    pub(crate) fn object(&mut self, key: &str) -> Option<&'a Map<String, Value>> {
        match self.raw(key)? {
            Value::Object(value) => Some(value),
            other => {
                self.issue(key, &format!("expected an object, found {}", kind(other)));
                None
            }
        }
    }

    /// A list of strings; an item of another type is dropped alone.
    pub(crate) fn strings(&mut self, key: &str) -> Vec<String> {
        let Some(raw) = self.raw(key) else {
            return Vec::new();
        };
        let Value::Array(items) = raw else {
            self.issue(
                key,
                &format!("expected a list of strings, found {}", kind(raw)),
            );
            return Vec::new();
        };
        let mut strings = Vec::new();
        for (index, item) in items.iter().enumerate() {
            match item {
                Value::String(value) => strings.push(value.clone()),
                other => self.issue(
                    &format!("{key}[{index}]"),
                    &format!("expected a string, found {}", kind(other)),
                ),
            }
        }
        strings
    }
}

pub(crate) fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

pub(crate) fn non_empty_trimmed(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}
