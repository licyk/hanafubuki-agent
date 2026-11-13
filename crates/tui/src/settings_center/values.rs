// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What a value reads as in a row (Codex's `[x] On`, a choice's label, a
//! number), the layer that set it, and the words the tabs are named by.

use hanafubuki_agent::{
    SettingCondition, SettingKind, SettingSource, SettingSpec, SettingValue, SettingsGroup,
};

/// The tabs, in their order.
pub(super) const GROUPS: [SettingsGroup; 9] = [
    SettingsGroup::General,
    SettingsGroup::Model,
    SettingsGroup::Tools,
    SettingsGroup::Memory,
    SettingsGroup::Context,
    SettingsGroup::Network,
    SettingsGroup::Interface,
    SettingsGroup::Plugins,
    SettingsGroup::Advanced,
];

pub(super) fn group_label(group: SettingsGroup) -> &'static str {
    match group {
        SettingsGroup::General => "General",
        SettingsGroup::Model => "Model",
        SettingsGroup::Tools => "Tools",
        SettingsGroup::Memory => "Memory",
        SettingsGroup::Context => "Context",
        SettingsGroup::Network => "Network",
        SettingsGroup::Interface => "Interface",
        SettingsGroup::Plugins => "Plugins",
        SettingsGroup::Advanced => "Advanced",
    }
}

/// `value` as a row shows it; `secret_set` for a secret field that holds a
/// value the catalog does not return.
pub(super) fn value_text(
    spec: &SettingSpec,
    value: Option<&SettingValue>,
    secret_set: bool,
) -> String {
    if secret_set {
        return "••••".to_string();
    }
    let Some(value) = value else {
        return "(unset)".to_string();
    };
    match (&spec.kind, value) {
        (_, SettingValue::Boolean(true)) => "[x] On".to_string(),
        (_, SettingValue::Boolean(false)) => "[ ] Off".to_string(),
        (
            SettingKind::Choice { values } | SettingKind::ChoiceOrText { values, .. },
            SettingValue::Text(chosen),
        ) => values
            .iter()
            .find(|choice| choice.value == *chosen)
            .map_or_else(|| chosen.clone(), |choice| choice.label.clone()),
        (_, SettingValue::Integer(value)) => value.to_string(),
        (_, SettingValue::Number(value)) => value.to_string(),
        (_, SettingValue::Text(text)) if text.is_empty() => "(empty)".to_string(),
        (_, SettingValue::Text(text)) => without_userinfo(text),
        (_, SettingValue::TextList(items)) if items.is_empty() => "(none)".to_string(),
        (_, SettingValue::TextList(items)) => items.join(", "),
        (_, SettingValue::Null) => "(unset)".to_string(),
    }
}

/// The text an inline editor starts with.
pub(super) fn editable_text(value: Option<&SettingValue>) -> String {
    match value {
        Some(SettingValue::Integer(value)) => value.to_string(),
        Some(SettingValue::Number(value)) => value.to_string(),
        Some(SettingValue::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

/// The layer above the user's file that sets a value, as a row names it;
/// `None` for the user's file and the default.
pub(super) fn source_text(source: &SettingSource) -> Option<String> {
    match source {
        SettingSource::Default | SettingSource::User => None,
        SettingSource::Profile { name } => Some(format!("profile {name}")),
        SettingSource::Project { .. } => Some("project".to_string()),
        SettingSource::Override => Some("-c".to_string()),
    }
}

/// Whether `condition` holds, the value of its key read by `value_of`.
pub(super) fn holds(
    condition: Option<&SettingCondition>,
    value_of: impl Fn(&str) -> Option<SettingValue>,
) -> bool {
    condition.is_none_or(|condition| value_of(&condition.key).as_ref() == Some(&condition.equals))
}

/// A URL's credentials hidden (`http://***@proxy:8080`); other text as it is.
fn without_userinfo(text: &str) -> String {
    let Some((scheme, rest)) = text.split_once("://") else {
        return text.to_string();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    match authority.rsplit_once('@') {
        Some((_, host)) => format!("{scheme}://***@{host}{}", &rest[authority.len()..]),
        None => text.to_string(),
    }
}
