//! Reading one option's value, tolerantly for what the settings hold and
//! strictly for a write.

use serde_json::{Map, Value};

use super::{
    PluginOption, PluginOptionKind, PluginOptionSchema, PluginOptionValue, ResolvedPluginOption,
    ResolvedPluginOptions,
};

/// Where an installed plugin's settings are set: they are layered as one
/// table, so the layer of a key is not known.
const GUEST_SETTINGS_LAYER: &str = "config.toml";

/// Why a value is not used as it is.
enum Problem {
    /// Out of its bounds: `clamped` is used instead.
    OutOfRange {
        clamped: PluginOptionValue,
        bounds: String,
    },
    Unusable(String),
}

impl PluginOption {
    /// `raw` as this option's value, or why it cannot be one: what a write
    /// checks before anything is written.
    pub fn check(&self, raw: &Value) -> Result<PluginOptionValue, String> {
        self.read(raw).map_err(|problem| match problem {
            Problem::OutOfRange { bounds, .. } => format!("`{}` is {bounds}", self.key),
            Problem::Unusable(reason) => format!("`{}` {reason}", self.key),
        })
    }

    fn read(&self, raw: &Value) -> Result<PluginOptionValue, Problem> {
        let unusable = |reason: &str| Err(Problem::Unusable(reason.to_string()));
        match &self.kind {
            PluginOptionKind::Boolean => match raw.as_bool() {
                Some(value) => Ok(PluginOptionValue::Boolean(value)),
                None => unusable("is not true or false"),
            },
            PluginOptionKind::Integer { minimum, maximum } => {
                let Some(value) = integer(raw) else {
                    return unusable("is not an integer");
                };
                let clamped = value.max(i128::from(*minimum)).min(i128::from(*maximum));
                let clamped = i64::try_from(clamped).unwrap_or(*maximum);
                if i128::from(clamped) == value {
                    Ok(PluginOptionValue::Integer(clamped))
                } else {
                    Err(Problem::OutOfRange {
                        clamped: PluginOptionValue::Integer(clamped),
                        bounds: format!("between {minimum} and {maximum}"),
                    })
                }
            }
            PluginOptionKind::Number { minimum, maximum } => {
                let Some(value) = raw.as_f64() else {
                    return unusable("is not a number");
                };
                let clamped = value.max(*minimum).min(*maximum);
                if clamped == value {
                    Ok(PluginOptionValue::Number(value))
                } else {
                    Err(Problem::OutOfRange {
                        clamped: PluginOptionValue::Number(clamped),
                        bounds: format!("between {minimum} and {maximum}"),
                    })
                }
            }
            PluginOptionKind::Text { maximum_chars, .. } => match raw.as_str() {
                Some(text) if text.chars().count() <= *maximum_chars => {
                    Ok(PluginOptionValue::Text(text.to_string()))
                }
                Some(_) => Err(Problem::Unusable(format!(
                    "is longer than {maximum_chars} characters"
                ))),
                None => unusable("is not a string"),
            },
            PluginOptionKind::Choice { values } => match raw.as_str() {
                Some(text) if values.iter().any(|value| value == text) => {
                    Ok(PluginOptionValue::Text(text.to_string()))
                }
                _ => Err(Problem::Unusable(format!(
                    "is not one of {}",
                    values.join(", ")
                ))),
            },
            PluginOptionKind::TextList {
                maximum_items,
                maximum_item_chars,
            } => {
                let Some(items) = raw.as_array() else {
                    return unusable("is not a list of strings");
                };
                if items.len() > *maximum_items {
                    return Err(Problem::Unusable(format!(
                        "has more than {maximum_items} items"
                    )));
                }
                let mut texts = Vec::with_capacity(items.len());
                for item in items {
                    match item.as_str() {
                        Some(text) if text.chars().count() <= *maximum_item_chars => {
                            texts.push(text.to_string())
                        }
                        Some(_) => {
                            return Err(Problem::Unusable(format!(
                                "has an item longer than {maximum_item_chars} characters"
                            )))
                        }
                        None => return unusable("is not a list of strings"),
                    }
                }
                Ok(PluginOptionValue::TextList(texts))
            }
        }
    }
}

impl PluginOptionSchema {
    pub fn option(&self, key: &str) -> Option<&PluginOption> {
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
                Err(Problem::OutOfRange { clamped, bounds }) => {
                    resolved.issues.push(format!(
                        "{name} is {raw}, which is not {bounds}; {} is used",
                        clamped.to_json()
                    ));
                    Some(clamped)
                }
                Err(Problem::Unusable(reason)) => {
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

/// A JSON integer, or a number with no fraction (`1.0`), widened so no
/// value overflows before it is clamped.
fn integer(raw: &Value) -> Option<i128> {
    if let Some(value) = raw.as_i64() {
        return Some(i128::from(value));
    }
    if let Some(value) = raw.as_u64() {
        return Some(i128::from(value));
    }
    let value = raw.as_f64()?;
    (value.fract() == 0.0 && value.abs() < 1e30).then_some(value as i128)
}
