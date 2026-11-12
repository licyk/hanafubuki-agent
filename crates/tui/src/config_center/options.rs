// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The generic "Plugin options" form: a field for each option of any
//! plugin's schema (a toggle, a bounded number, a choice, a text, masked
//! when it is secret, or a list of texts), and the values it saves, each
//! checked as the write will check it.

use hanafubuki_agent::{
    PluginOptionSchema, ResolvedPluginOption, SettingKind, SettingSpec, SettingValue,
    MAX_PLUGIN_OPTIONS,
};
use serde_json::{Map, Value};

use crate::agent_bridge::ConfigState;
use crate::bottom_pane::{FieldId, FormItem, FormView};

/// The form's Save button.
const SAVE_OPTIONS: FieldId = "save_options";

/// One field per option, by its place in the schema.
const FIELDS: [FieldId; MAX_PLUGIN_OPTIONS] = [
    "option_0",
    "option_1",
    "option_2",
    "option_3",
    "option_4",
    "option_5",
    "option_6",
    "option_7",
    "option_8",
    "option_9",
    "option_10",
    "option_11",
    "option_12",
    "option_13",
    "option_14",
    "option_15",
    "option_16",
    "option_17",
    "option_18",
    "option_19",
    "option_20",
    "option_21",
    "option_22",
    "option_23",
    "option_24",
    "option_25",
    "option_26",
    "option_27",
    "option_28",
    "option_29",
    "option_30",
    "option_31",
    "option_32",
    "option_33",
    "option_34",
    "option_35",
    "option_36",
    "option_37",
    "option_38",
    "option_39",
    "option_40",
    "option_41",
    "option_42",
    "option_43",
    "option_44",
    "option_45",
    "option_46",
    "option_47",
    "option_48",
    "option_49",
    "option_50",
    "option_51",
    "option_52",
    "option_53",
    "option_54",
    "option_55",
    "option_56",
    "option_57",
    "option_58",
    "option_59",
    "option_60",
    "option_61",
    "option_62",
    "option_63",
];

/// The field of the option at `index` of a schema.
#[cfg(test)]
pub(super) fn option_field(index: usize) -> FieldId {
    FIELDS[index]
}

/// What an option without a value shows in a choice.
const UNSET: &str = "unset";

/// The form over `schema`, starting from `values` (one per option, as the
/// plugin's status has them).
pub(crate) fn options_form(
    title: &str,
    schema: &PluginOptionSchema,
    values: &[ResolvedPluginOption],
) -> FormView {
    let mut items = schema
        .options
        .iter()
        .zip(FIELDS)
        .map(|(option, id)| option_item(id, option, current(option, values)))
        .collect::<Vec<_>>();
    items.push(FormItem::action(SAVE_OPTIONS, "Save"));
    FormView::new(title, items).with_subtitle(
        "Each value is checked against the plugin's options before it is saved to the user's config.toml.",
    )
}

/// The options the form changed from `values`, each checked; `None`, with
/// the errors on their fields, when one does not fit.
pub(crate) fn changed_options(
    form: &mut FormView,
    schema: &PluginOptionSchema,
    values: &[ResolvedPluginOption],
) -> Option<Map<String, Value>> {
    form.clear_errors();
    let mut changed = Map::new();
    let mut fits = true;
    for (option, id) in schema.options.iter().zip(FIELDS) {
        let before = current(option, values);
        let Some(raw) = read(form, id, option, before.is_none()) else {
            continue;
        };
        let raw = match raw {
            Ok(raw) => raw,
            Err(error) => {
                form.set_error(id, error);
                fits = false;
                continue;
            }
        };
        match option.check(&raw) {
            Ok(value) => {
                if before.map(SettingValue::to_json) != Some(value.to_json()) {
                    changed.insert(option.key.clone(), value.to_json());
                }
            }
            Err(error) => {
                form.set_error(id, error);
                fits = false;
            }
        }
    }
    fits.then_some(changed)
}

/// `/config plugins`' link to the plugin manager.
pub(super) const OPEN_PLUGINS: FieldId = "open_plugins";

/// `/config plugins`: a link to each built-in plugin's options, by its place
/// among those with options, and one to the plugin manager.
pub(super) fn plugins_form(state: &ConfigState) -> FormView {
    let mut items = state
        .plugins
        .iter()
        .zip(FIELDS)
        .map(|(plugin, id)| FormItem::action(id, &format!("Options of {} ›", plugin.name)))
        .collect::<Vec<_>>();
    items.push(FormItem::action(
        OPEN_PLUGINS,
        "Plugins and packages, and a code plugin's options (/plugins) ›",
    ));
    FormView::new("Plugins & packages", items).with_subtitle(
        "Turning plugins on and off, installing and updating them is /plugins'; o there opens a plugin's options too.",
    )
}

/// The options form of the plugin a link of [`plugins_form`] names, and
/// the plugin's name.
pub(super) fn plugin_options_form(
    state: &ConfigState,
    link: FieldId,
) -> Option<(String, FormView)> {
    let index = FIELDS.iter().position(|id| *id == link)?;
    let plugin = state.plugins.get(index)?;
    let options = plugin.options.as_ref()?;
    let title = format!("Options of {}", plugin.name);
    let form = options_form(&title, &options.schema, &options.values);
    Some((plugin.name.clone(), form))
}

/// What saving the options form of the plugin `row` writes; `None`, with
/// the form's errors set, when a value does not fit.
pub(super) fn draft_plugin_options(
    state: &ConfigState,
    row: &str,
    form: &mut FormView,
) -> Option<super::ConfigChange> {
    let plugin = state.plugins.iter().find(|plugin| plugin.name == row)?;
    let options = plugin.options.as_ref()?;
    let changed = changed_options(form, &options.schema, &options.values)?;
    Some(super::ConfigChange::PluginOptions {
        row: row.to_string(),
        options: changed,
    })
}

/// Whether a layer above the user's file sets one of the options written
/// to another value than the one written.
pub(super) fn is_shadowed(state: &ConfigState, row: &str, written: &Map<String, Value>) -> bool {
    let Some(options) = state
        .plugins
        .iter()
        .find(|plugin| plugin.name == row)
        .and_then(|plugin| plugin.options.as_ref())
    else {
        return false;
    };
    written.iter().any(|(key, value)| {
        let effective = options
            .values
            .iter()
            .find(|option| option.key == *key)
            .and_then(|option| option.value.as_ref())
            .map(SettingValue::to_json);
        !value.is_null() && effective.as_ref() != Some(value)
    })
}

/// What an option is labelled: its summary, and its key, which the file
/// names it by.
fn label(option: &SettingSpec) -> String {
    if option.summary.is_empty() {
        option.key.clone()
    } else {
        format!("{} ({})", option.summary, option.key)
    }
}

fn current<'a>(
    option: &SettingSpec,
    values: &'a [ResolvedPluginOption],
) -> Option<&'a SettingValue> {
    values
        .iter()
        .find(|value| value.key == option.key)?
        .value
        .as_ref()
}

fn option_item(id: FieldId, option: &SettingSpec, value: Option<&SettingValue>) -> FormItem {
    let label = label(option);
    let text = |value: Option<&SettingValue>| match value {
        Some(SettingValue::Integer(value)) => value.to_string(),
        Some(SettingValue::Number(value)) => value.to_string(),
        Some(SettingValue::Text(value)) => value.clone(),
        Some(SettingValue::TextList(values)) => values.join(", "),
        Some(SettingValue::Boolean(value)) => value.to_string(),
        Some(SettingValue::Null) | None => String::new(),
    };
    match &option.kind {
        SettingKind::Boolean => match value {
            Some(SettingValue::Boolean(on)) => FormItem::toggle(id, &label, *on),
            _ => FormItem::choice(id, &label, &[UNSET, "on", "off"], 0),
        },
        SettingKind::Integer { minimum, maximum } => {
            FormItem::text(id, &label, &text(value)).placeholder(&bounds(*minimum, *maximum))
        }
        SettingKind::Number { minimum, maximum } => {
            FormItem::text(id, &label, &text(value)).placeholder(&bounds(*minimum, *maximum))
        }
        SettingKind::Text { secret, .. } => {
            let item = FormItem::text(id, &label, &text(value));
            if *secret {
                item.secret()
            } else {
                item
            }
        }
        SettingKind::Choice { values } => {
            let mut choices = values
                .iter()
                .map(|choice| choice.label.as_str())
                .collect::<Vec<_>>();
            let selected = match value {
                Some(SettingValue::Text(chosen)) => values
                    .iter()
                    .position(|choice| choice.value == *chosen)
                    .unwrap_or(0),
                _ => {
                    choices.insert(0, UNSET);
                    0
                }
            };
            FormItem::choice(id, &label, &choices, selected)
        }
        SettingKind::Path { .. } => FormItem::text(id, &label, &text(value)),
        SettingKind::TextList { .. } => {
            FormItem::text(id, &label, &text(value)).placeholder("one, two, three")
        }
    }
}

/// `minimum to maximum`, without the bound that is none.
fn bounds<T: std::fmt::Display>(minimum: Option<T>, maximum: Option<T>) -> String {
    match (minimum, maximum) {
        (Some(minimum), Some(maximum)) => format!("{minimum} to {maximum}"),
        (Some(minimum), None) => format!("at least {minimum}"),
        (None, Some(maximum)) => format!("at most {maximum}"),
        (None, None) => String::new(),
    }
}

/// The field's value as JSON; `None` when it is left unset, an error when
/// it does not read as its kind. A toggle or a choice for an option that
/// had no value (`unset`) starts with an `unset` entry.
fn read(
    form: &FormView,
    id: FieldId,
    option: &SettingSpec,
    unset: bool,
) -> Option<Result<Value, String>> {
    let text = form.text(id).trim();
    let chosen = |count: usize| match (unset, form.choice(id)) {
        (true, 0) => None,
        (true, index) => Some(index - 1).filter(|index| *index < count),
        (false, index) => Some(index),
    };
    match &option.kind {
        SettingKind::Boolean if !unset => Some(Ok(Value::Bool(form.toggle(id)))),
        SettingKind::Boolean => chosen(2).map(|index| Ok(Value::Bool(index == 0))),
        SettingKind::Integer { .. }
        | SettingKind::Number { .. }
        | SettingKind::Text { .. }
        | SettingKind::Path { .. }
            if text.is_empty() =>
        {
            None
        }
        SettingKind::Integer { .. } => Some(
            text.parse::<i64>()
                .map(Value::from)
                .map_err(|_| "Type an integer.".to_string()),
        ),
        SettingKind::Number { .. } => Some(
            text.parse::<f64>()
                .map(Value::from)
                .map_err(|_| "Type a number.".to_string()),
        ),
        SettingKind::Text { .. } | SettingKind::Path { .. } => {
            Some(Ok(Value::String(form.text(id).to_string())))
        }
        SettingKind::Choice { values } => chosen(values.len())
            .and_then(|index| values.get(index))
            .map(|choice| Ok(Value::String(choice.value.clone()))),
        SettingKind::TextList { .. } if text.is_empty() && unset => None,
        SettingKind::TextList { .. } => Some(Ok(Value::from(
            text.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>(),
        ))),
    }
}
