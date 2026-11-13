//! How the interface names what is known of a model: where a fact came
//! from, what a model list's entry reports, and the reasoning efforts
//! offered for it.

use hanafubuki_agent::{ModelCatalogEntry, ModelMetadataSource, SettingChoice};

use crate::history_cell::format_tokens_compact;

/// Where a fact came from, as the interface names it.
pub(crate) fn source_name(source: ModelMetadataSource) -> &'static str {
    match source {
        ModelMetadataSource::Configured => "configured",
        ModelMetadataSource::Learned => "learned",
        ModelMetadataSource::Reported => "reported",
        ModelMetadataSource::Catalog => "models.dev",
        ModelMetadataSource::BuiltIn => "built-in",
    }
}

/// What a listing's entry reports, for a model list: its window, and
/// `vision` and `reasoning` when it reports them (`200K · vision`); none
/// when it reports none of them.
pub(crate) fn entry_badges(entry: &ModelCatalogEntry) -> Option<String> {
    let badges: Vec<String> = entry
        .context_window
        .map(format_tokens_compact)
        .into_iter()
        .chain((entry.image_input == Some(true)).then(|| "vision".to_string()))
        .chain((entry.reasoning == Some(true)).then(|| "reasoning".to_string()))
        .collect();
    (!badges.is_empty()).then(|| badges.join(" · "))
}

/// What an effort option sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EffortValue {
    Level(String),
    /// Unset: the Provider's default.
    Default,
    /// One typed.
    Custom,
}

/// One option of `/effort` and of the settings center's effort row.
#[derive(Clone, Debug)]
pub(crate) struct EffortOption {
    pub(crate) value: EffortValue,
    pub(crate) label: String,
    pub(crate) description: Option<&'static str>,
    pub(crate) current: bool,
}

/// The efforts offered for a model, the same wherever one is chosen: its
/// known levels (`levels`, `reasoning_effort_choices`), then Default and
/// Custom. The `current` effort is marked; one off the levels is the
/// custom one, named.
pub(crate) fn effort_options(levels: &[SettingChoice], current: Option<&str>) -> Vec<EffortOption> {
    let listed = levels
        .iter()
        .any(|level| Some(level.value.as_str()) == current);
    let custom = current.filter(|_| !listed);
    levels
        .iter()
        .map(|level| EffortOption {
            value: EffortValue::Level(level.value.clone()),
            label: level.label.clone(),
            description: None,
            current: Some(level.value.as_str()) == current,
        })
        .chain([
            EffortOption {
                value: EffortValue::Default,
                label: "Default".to_string(),
                description: Some("unset: the Provider's default"),
                current: current.is_none(),
            },
            EffortOption {
                value: EffortValue::Custom,
                label: custom.map_or_else(
                    || "Custom…".to_string(),
                    |effort| format!("Custom: {effort}"),
                ),
                description: Some("type an effort"),
                current: custom.is_some(),
            },
        ])
        .collect()
}

#[cfg(test)]
mod tests;
