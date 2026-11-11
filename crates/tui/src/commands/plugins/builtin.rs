// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The Built-in tab: the plugins compiled into the Agent, each with its
//! state and summary, turned on and off, and the ones with options.

use hanafubuki_agent::{AgentPluginState, AgentPluginStatus};

use super::pages::{call, After, Choice, Row};
use crate::agent_bridge::{PluginsCall, PluginsState};
use crate::bottom_pane::SelectionItem;

pub(super) fn builtin_rows(state: &PluginsState) -> Vec<Row> {
    state
        .builtins
        .iter()
        .map(|plugin| {
            let mut description = builtin_state(plugin).to_string();
            if !plugin.summary.is_empty() {
                description.push_str(&format!(" · {}", plugin.summary));
            }
            if plugin.options.is_some() {
                description.push_str(" · has options (o)");
            }
            if plugin.lock.is_some() {
                description.push_str(" · always on");
            }
            if let Some(error) = plugin.error.as_ref().or(plugin.warning.as_ref()) {
                description.push_str(&format!(" · {error}"));
            }
            let item = SelectionItem {
                name: plugin.name.clone(),
                description: Some(description),
                ..Default::default()
            };
            (item, builtin_toggle(plugin))
        })
        .collect()
}

/// Turning a built-in plugin on or off; a core one cannot be.
pub(super) fn builtin_toggle(plugin: &AgentPluginStatus) -> Option<Choice> {
    if plugin.lock.is_some() {
        return None;
    }
    let enabled = !plugin.enabled;
    Some(call(
        PluginsCall::SetBuiltinEnabled {
            name: plugin.name.clone(),
            enabled,
        },
        if enabled {
            "Turning it on…"
        } else {
            "Turning it off…"
        },
        After::Stay,
    ))
}

fn builtin_state(plugin: &AgentPluginStatus) -> &'static str {
    match plugin.state {
        AgentPluginState::Pending => "starting",
        AgentPluginState::Active => "on",
        AgentPluginState::Failed => "failed",
        AgentPluginState::Disabled => "off",
        AgentPluginState::Disposed => "stopped",
    }
}
