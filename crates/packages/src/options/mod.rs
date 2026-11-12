//! Plugin options: the settings a plugin declares, which the configuration
//! center edits with one generic form.
//!
//! An option is a [`SettingSpec`] whose key is relative to the plugin's
//! table. A built-in row declares its [`PluginOptionSchema`] in code; an
//! installed plugin's code declares a JSON Schema (`configSchema`), which is
//! lowered to the same vocabulary once, when the package is read
//! ([`lower_config_schema`]). Values are read tolerantly, each on its own
//! ([`PluginOptionSchema::resolve`]): one that does not fit takes its
//! default with an issue, and an integer or a number out of range is clamped
//! with a note. A write is strict ([`SettingSpec::check`]).

mod lower;
mod resolve;

use hanafubuki_agent_foundation::settings::{SettingSpec, SettingValue};
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
    /// A built-in row's options always have a default; a `configSchema`
    /// property without a `default` has none, and is then left out of what
    /// the guest receives.
    pub options: Vec<SettingSpec>,
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
    pub value: Option<SettingValue>,
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

    pub fn value(&self, key: &str) -> Option<&SettingValue> {
        self.values
            .iter()
            .find(|option| option.key == key)?
            .value
            .as_ref()
    }
}
