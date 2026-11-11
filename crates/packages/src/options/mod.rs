//! Plugin options: the typed settings a plugin declares, which the
//! configuration center edits with one generic form.
//!
//! A built-in row declares its [`PluginOptionSchema`] in code; an installed
//! plugin's code declares a JSON Schema (`configSchema`), which is lowered to
//! the same vocabulary once, when the package is read
//! ([`lower_config_schema`]). Values are read tolerantly, each on its own
//! ([`PluginOptionSchema::resolve`]): one that does not fit takes its
//! default with an issue, and an integer or a number out of range is clamped
//! with a note. A write is strict ([`PluginOption::check`]).

mod lower;
mod value;

use serde::Serialize;
use serde_json::{Map, Value};

pub use lower::lower_config_schema;

/// The most options one schema has; a `configSchema` with more leaves the
/// rest out, each with an issue.
pub const MAX_PLUGIN_OPTIONS: usize = 64;

/// A plugin's options, in the order a form shows them.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginOptionSchema {
    pub options: Vec<PluginOption>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginOption {
    /// The option's key in its table, camelCase.
    pub key: String,
    /// One line for the form.
    pub summary: String,
    pub kind: PluginOptionKind,
    /// What it is when nothing usable sets it. A built-in row's options
    /// always have one; a `configSchema` property without a `default` has
    /// none, and is then left out of what the guest receives.
    pub default: Option<PluginOptionValue>,
}

/// What values an option takes. Bounds are inclusive.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum PluginOptionKind {
    Boolean,
    Integer {
        minimum: i64,
        maximum: i64,
    },
    Number {
        minimum: f64,
        maximum: f64,
    },
    Text {
        maximum_chars: usize,
        /// Drawn as dots by a form.
        secret: bool,
    },
    Choice {
        values: Vec<String>,
    },
    TextList {
        maximum_items: usize,
        maximum_item_chars: usize,
    },
}

/// One option's value, as its kind reads it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum PluginOptionValue {
    Boolean(bool),
    Integer(i64),
    Number(f64),
    /// A text or a choice.
    Text(String),
    TextList(Vec<String>),
}

impl PluginOptionValue {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Boolean(value) => Value::Bool(*value),
            Self::Integer(value) => Value::from(*value),
            Self::Number(value) => Value::from(*value),
            Self::Text(value) => Value::String(value.clone()),
            Self::TextList(values) => Value::from(values.clone()),
        }
    }
}

/// A plugin's options as its settings set them.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPluginOptions {
    /// One per option of the schema, in its order.
    pub values: Vec<ResolvedPluginOption>,
    /// What could not be used as it was set, each naming where it was set
    /// and its key.
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPluginOption {
    pub key: String,
    /// `None` only for an option without a default that nothing usable
    /// sets.
    pub value: Option<PluginOptionValue>,
    /// Where the value was set; `None` when it is the default.
    pub layer: Option<String>,
}

impl ResolvedPluginOptions {
    /// The values as one object, each option that has one under its key.
    pub fn to_object(&self) -> Map<String, Value> {
        self.values
            .iter()
            .filter_map(|option| Some((option.key.clone(), option.value.as_ref()?.to_json())))
            .collect()
    }

    pub fn value(&self, key: &str) -> Option<&PluginOptionValue> {
        self.values
            .iter()
            .find(|option| option.key == key)?
            .value
            .as_ref()
    }
}
