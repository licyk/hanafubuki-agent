//! Reading a plugin's options from what its settings hold, each on its
//! own and tolerantly.

use hanafubuki_agent_foundation::settings::{SettingProblem, SettingSpec};
use serde_json::{Map, Value};

use super::{PluginOptionSchema, ResolvedPluginOption, ResolvedPluginOptions};

/// Where an installed plugin's settings are set: they are layered as one
/// table, so the layer of a key is not known.
const GUEST_SETTINGS_LAYER: &str = "config.toml";

impl PluginOptionSchema {
    pub fn option(&self, key: &str) -> Option<&SettingSpec> {
        self.options.iter().find(|option| option.key == key)
    }

    /// Each option as `set` sets it: `set` gives the value of a key and the
    /// layer that set it, which the issues name with `table`
    /// (`pluginOptions.subagents`). A value that does not fit takes the
    /// default; an integer or a number out of range is clamped. Keys
    /// `set` holds that the schema does not are the caller's.
    pub fn resolve(
        &self,
        table: &str,
        set: impl Fn(&str) -> Option<(Value, String)>,
    ) -> ResolvedPluginOptions {
        let mut resolved = ResolvedPluginOptions::default();
        for option in &self.options {
            let default = ResolvedPluginOption {
                key: option.key.clone(),
                value: option.default.clone(),
                layer: None,
            };
            let Some((raw, layer)) = set(&option.key) else {
                resolved.values.push(default);
                continue;
            };
            let name = format!("{layer}: {table}.{}", option.key);
            let value = match option.read(&raw) {
                Ok(value) => Some(value),
                Err(SettingProblem::OutOfRange { clamped, bounds }) => {
                    resolved.issues.push(format!(
                        "{name} is {raw}, which is not {bounds}; {} is used",
                        clamped.to_json()
                    ));
                    Some(clamped)
                }
                Err(SettingProblem::Unusable(reason)) => {
                    let fallback = match &option.default {
                        Some(default) => format!("its default {} is used", default.to_json()),
                        None => "it is left out".to_string(),
                    };
                    resolved.issues.push(format!("{name} {reason}; {fallback}"));
                    resolved.values.push(default);
                    continue;
                }
            };
            resolved.values.push(ResolvedPluginOption {
                key: option.key.clone(),
                value,
                layer: Some(layer),
            });
        }
        resolved
    }

    /// An installed plugin's `settings`, the table `table` of `config.toml`
    /// (`plugins."<id>".settings`), with each key the schema describes
    /// checked: one that does not fit takes its default, or is left out
    /// when it has none; one out of range is clamped. A key the settings
    /// leave out stays out, and one the schema does not describe is kept as
    /// it is: what the lowering could not read constrains nothing.
    pub fn check_settings(
        &self,
        table: &str,
        settings: &Map<String, Value>,
    ) -> (Map<String, Value>, ResolvedPluginOptions) {
        let resolved = self.resolve(table, |key| {
            Some((settings.get(key)?.clone(), GUEST_SETTINGS_LAYER.to_string()))
        });
        let mut checked = settings.clone();
        for option in &resolved.values {
            if !settings.contains_key(&option.key) {
                continue;
            }
            match &option.value {
                Some(value) => checked.insert(option.key.clone(), value.to_json()),
                None => checked.remove(&option.key),
            };
        }
        (checked, resolved)
    }
}
