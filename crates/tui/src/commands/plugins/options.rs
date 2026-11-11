// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The options form `o` opens: a built-in plugin's options
//! (`[pluginOptions.<plugin>]`) or a code plugin's settings, as its
//! `configSchema` lowers to options, in the configuration center's generic
//! form.

use hanafubuki_agent::{AgentPackageStatus, AgentPluginOptionsStatus, AgentPluginStatus};
use serde_json::Value;

use super::pages::OptionsOf;
use crate::agent_bridge::{PluginsCall, PluginsState};
use crate::bottom_pane::FormView;
use crate::config_center::{changed_options, options_form};

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
        // A code plugin's settings are written whole: the keys its options
        // do not describe stay.
        OptionsOf::Package(id) => {
            let mut settings = package(state, id)?.guest.as_ref()?.settings.clone();
            settings.extend(changed);
            PluginsCall::SetSettings {
                id: id.clone(),
                settings: Value::Object(settings),
            }
        }
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
