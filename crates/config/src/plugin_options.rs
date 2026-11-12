//! `[pluginOptions]`: a built-in plugin's options, keyed by its name, then
//! by option (`[pluginOptions.subagents] maximumRunningAgents = 4`).
//!
//! The configuration does not know which plugins have which options: their
//! descriptors are the facade's. So the table is kept as the layers set it,
//! merged per plugin and key, each value with the layer that set it, and a
//! plugin's options are resolved against its schema when they are read
//! ([`PluginOptionsSettings::resolve`]). A plugin no descriptor names keeps
//! its table, which means nothing, as `disabledPlugins` keeps a name. A
//! write is checked against the schema before anything is written
//! ([`crate::AgentConfig::set_plugin_options`]).

use std::collections::BTreeMap;

use hanafubuki_agent_packages::{PluginOptionSchema, ResolvedPluginOptions};
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use crate::keys::{parse_key, PACKAGE_SETTINGS, PLUGINS, PLUGIN_OPTIONS};
use crate::sections::SettingsRead;
use crate::settings::is_plugin_name;

/// The `[pluginOptions]` table as the layers set it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginOptionsSettings {
    rows: BTreeMap<String, BTreeMap<String, PluginOptionSetting>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PluginOptionSetting {
    value: Value,
    /// The layer that set it, as a warning names it.
    layer: String,
}

/// The values alone: where each was set is not written.
impl Serialize for PluginOptionsSettings {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.rows.iter().map(|(row, options)| {
            let options = options
                .iter()
                .map(|(key, setting)| (key.clone(), setting.value.clone()))
                .collect::<Map<_, _>>();
            (row, options)
        }))
    }
}

impl PluginOptionsSettings {
    /// The options of the plugin `row` against its `schema`. A key its
    /// table sets that the schema does not have is kept in the file and
    /// ignored, with an issue.
    pub fn resolve(&self, row: &str, schema: &PluginOptionSchema) -> ResolvedPluginOptions {
        let table = format!("{PLUGIN_OPTIONS}.{row}");
        let set = self.rows.get(row);
        let mut resolved = schema.resolve(&table, |key| {
            let setting = set?.get(key)?;
            Some((setting.value.clone(), setting.layer.clone()))
        });
        for (key, setting) in set.into_iter().flatten() {
            if schema.option(key).is_none() {
                resolved.issues.push(format!(
                    "{}: {table}.{key} is not an option of the plugin {row}; it is kept and ignored",
                    setting.layer
                ));
            }
        }
        resolved
    }

    /// The value `key` of the plugin `row` is set to, as the layers set it.
    pub fn value(&self, row: &str, key: &str) -> Option<&Value> {
        Some(&self.rows.get(row)?.get(key)?.value)
    }
}

/// The table in `fields`, which `layer` names, over the layers below: a key
/// it sets replaces the one below. A plugin's entry that is not a table, or
/// whose name cannot be a plugin's, is left out and reported as an unknown
/// key, so the file keeps it as written.
pub(crate) fn decode_plugin_options(
    read: &mut SettingsRead<'_>,
    below: &PluginOptionsSettings,
    layer: &str,
) -> PluginOptionsSettings {
    let mut options = below.clone();
    let fields = read.fields();
    let Some(raw) = fields.raw(PLUGIN_OPTIONS) else {
        return options;
    };
    let Some(table) = raw.as_object() else {
        fields.invalid(PLUGIN_OPTIONS, None);
        return options;
    };
    for (row, entry) in table {
        let Some(entry) = entry.as_object().filter(|_| is_plugin_name(row)) else {
            continue;
        };
        let set = options.rows.entry(row.clone()).or_default();
        for (key, value) in entry {
            set.insert(
                key.clone(),
                PluginOptionSetting {
                    value: value.clone(),
                    layer: layer.to_string(),
                },
            );
            // Whether the plugin can use it is its schema's to say.
            read.mark(&format!("{PLUGIN_OPTIONS}.{row}.{key}"));
        }
    }
    options
}

/// The table of `row` a write of `patch` makes: each value checked against
/// `schema`, `null` kept to remove its key, which returns it to its default.
pub(crate) fn checked_patch(
    row: &str,
    schema: &PluginOptionSchema,
    patch: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    if !is_plugin_name(row) {
        return Err(format!("`{row}` cannot be a plugin's name"));
    }
    patch
        .iter()
        .map(|(key, value)| {
            let option = schema
                .option(key)
                .ok_or_else(|| format!("the plugin {row} has no option `{key}`"))?;
            let value = match value {
                Value::Null => Value::Null,
                value => option.check(value)?.to_json(),
            };
            Ok((key.clone(), value))
        })
        .collect()
}

/// An option named by a dotted key (`pluginOptions.<row>.<key>`), as
/// `config get` and `config set` name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginOptionKey {
    pub row: String,
    pub key: String,
}

impl PluginOptionKey {
    /// The option `key` names; `None` when it names something else.
    pub fn parse(key: &str) -> Result<Option<Self>, String> {
        Ok(match parse_key(key)?.as_slice() {
            [first, row, key] if first == PLUGIN_OPTIONS => Some(Self {
                row: row.clone(),
                key: key.clone(),
            }),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests;

/// A setting of an installed plugin's code named by a dotted key
/// (`plugins."<id>".settings.<option>`), as `config set` names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageSettingKey {
    pub id: String,
    pub option: String,
}

impl PackageSettingKey {
    /// The setting `key` names; `None` when it names something else.
    pub fn parse(key: &str) -> Result<Option<Self>, String> {
        Ok(match parse_key(key)?.as_slice() {
            [first, id, settings, option] if first == PLUGINS && settings == PACKAGE_SETTINGS => {
                Some(Self {
                    id: id.clone(),
                    option: option.clone(),
                })
            }
            _ => None,
        })
    }
}
