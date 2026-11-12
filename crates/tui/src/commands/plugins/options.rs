// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The options form `o` opens: a built-in plugin's options
//! (`[pluginOptions.<plugin>]`) or a code plugin's settings, as its
//! `configSchema` lowers to options: a field for each option of any
//! plugin's schema (a toggle, a bounded number, a choice, a text, masked
//! when it is secret, or a list of texts), and the values it saves, each
//! checked as the write will check it.

use hanafubuki_agent::{
    AgentPackageStatus, AgentPluginOptionsStatus, AgentPluginStatus, PluginOptionSchema,
    ResolvedPluginOption, SettingKind, SettingSpec, SettingValue, MAX_PLUGIN_OPTIONS,
};
use serde_json::{Map, Value};

use super::pages::OptionsOf;
use crate::agent_bridge::{PluginsCall, PluginsState};
use crate::bottom_pane::{FieldId, FormItem, FormView};

/// The form's Save button.
const SAVE_OPTIONS: FieldId = "save_options";

/// The options of a built-in plugin, when it has some.
pub(super) fn of_builtin(plugin: &AgentPluginStatus) -> Option<OptionsOf> {
    plugin
        .options
        .as_ref()
        .map(|_| OptionsOf::Builtin(plugin.name.clone()))
}

/// The options of a code plugin, when its `configSchema` lowers to some.
pub(super) fn of_package(package: &AgentPackageStatus) -> Option<OptionsOf> {
    let options = package.guest.as_ref()?.options.as_ref()?;
    (!options.schema.options.is_empty()).then(|| OptionsOf::Package(package.id.clone()))
}

/// The form of `target`'s options; `None` when it is gone.
pub(super) fn form(state: &PluginsState, target: &OptionsOf) -> Option<FormView> {
    let (title, options) = options(state, target)?;
    Some(options_form(&title, &options.schema, &options.values))
}

/// What saving the form asks of the Agent; `None`, with the form's errors
/// set, when a value does not fit.
pub(super) fn save(
    form: &mut FormView,
    state: &PluginsState,
    target: &OptionsOf,
) -> Option<PluginsCall> {
    let (_, options) = options(state, target)?;
    let changed = changed_options(form, &options.schema, &options.values)?;
    Some(match target {
        OptionsOf::Builtin(name) => PluginsCall::SetBuiltinOptions {
            name: name.clone(),
            options: changed,
        },
        OptionsOf::Package(id) => PluginsCall::SetPackageOptions {
            id: id.clone(),
            options: changed,
        },
    })
}

fn options<'a>(
    state: &'a PluginsState,
    target: &OptionsOf,
) -> Option<(String, &'a AgentPluginOptionsStatus)> {
    match target {
        OptionsOf::Builtin(name) => {
            let plugin = state.builtins.iter().find(|plugin| plugin.name == *name)?;
            Some((format!("Options of {name}"), plugin.options.as_ref()?))
        }
        OptionsOf::Package(id) => {
            let package = package(state, id)?;
            let options = package.guest.as_ref()?.options.as_ref()?;
            Some((format!("Options of {}", package.display_name), options))
        }
    }
}

fn package<'a>(state: &'a PluginsState, id: &str) -> Option<&'a AgentPackageStatus> {
    state.packages.iter().find(|package| package.id == id)
}

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
pub(super) fn options_form(
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
pub(super) fn changed_options(
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
