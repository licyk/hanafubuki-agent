use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::compaction::decode_compaction;
use super::memory::validate_memory;
use super::packages::{decode_marketplaces, decode_plugins};
use super::sandbox::{decode_sandbox_mode, validate_sandbox_workspace_write};
use super::web::{decode_search_provider, decode_searxng_base_url};
use super::{is_plugin_name, AgentSettings, MAX_AGENT_DISABLED_PLUGINS};
use crate::keys::DISABLED_PLUGINS;
use crate::plugin_options::decode_plugin_options;
use crate::sections::{no_rule, LayerRead, SettingsRead};

/// The settings `object` holds, each read by its spec; a missing or invalid
/// one takes its value in `defaults` (the defaults, or the layer below in a
/// layered configuration). `layer` names the layer `object` is, which each
/// plugin option it sets remembers.
pub(crate) fn decode_settings(
    object: &Map<String, Value>,
    defaults: &AgentSettings,
    layer: &str,
    report: &mut LayerRead,
) -> AgentSettings {
    let mut read = SettingsRead::new(object, report);
    let permission_mode = read.read("permissionMode", &defaults.permission_mode, no_rule);
    let sandbox_mode = decode_sandbox_mode(&mut read, defaults.sandbox_mode);
    let sandbox_workspace_write = read.read(
        "sandboxWorkspaceWrite",
        &defaults.sandbox_workspace_write,
        validate_sandbox_workspace_write,
    );
    let windows = read.read("windows", &defaults.windows, no_rule);
    let memory = read.read("memory", &defaults.memory, validate_memory);
    let disabled_plugins = decode_disabled_plugins(&mut read, &defaults.disabled_plugins);
    let wake_on_background_tasks = read.read(
        "wakeOnBackgroundTasks",
        &defaults.wake_on_background_tasks,
        no_rule,
    );
    let searxng_base_url = decode_searxng_base_url(&mut read, &defaults.searxng_base_url);
    let web_search_provider = decode_search_provider(
        &mut read,
        defaults.web_search_provider,
        searxng_base_url.is_some(),
    );
    let web_fetch_use_jina_reader = read.read(
        "webFetchUseJinaReader",
        &defaults.web_fetch_use_jina_reader,
        no_rule,
    );
    let web_fetch_resolve_through_proxy = read.read(
        "webFetchResolveThroughProxy",
        &defaults.web_fetch_resolve_through_proxy,
        no_rule,
    );
    let compaction = decode_compaction(&mut read, &defaults.compaction);
    let model_metadata = read.read("modelMetadata", &defaults.model_metadata, no_rule);
    let fields = read.fields();
    let plugins = decode_plugins(fields, &defaults.plugins);
    let marketplaces = decode_marketplaces(fields, &defaults.marketplaces);
    let plugin_options = decode_plugin_options(&mut read, &defaults.plugin_options, layer);
    AgentSettings {
        permission_mode,
        sandbox_mode,
        sandbox_workspace_write,
        windows,
        memory,
        disabled_plugins,
        wake_on_background_tasks,
        web_search_provider,
        searxng_base_url,
        web_fetch_use_jina_reader,
        web_fetch_resolve_through_proxy,
        compaction,
        model_metadata,
        plugins,
        marketplaces,
        plugin_options,
    }
}

/// A name that cannot be one is dropped alone: losing the whole set would
/// enable what the user turned off.
fn decode_disabled_plugins(
    read: &mut SettingsRead<'_>,
    default: &BTreeSet<String>,
) -> BTreeSet<String> {
    let fields = read.fields();
    let Some(raw) = fields.raw(DISABLED_PLUGINS) else {
        return default.clone();
    };
    let Some(items) = raw.as_array() else {
        fields.invalid(
            DISABLED_PLUGINS,
            Some("the disabled plugin list is unreadable; every plugin is enabled until plugins are turned off again"),
        );
        read.mark(DISABLED_PLUGINS);
        return BTreeSet::new();
    };
    let mut kept = BTreeSet::new();
    for (index, value) in items.iter().enumerate() {
        let field = format!("{DISABLED_PLUGINS}[{index}]");
        match value.as_str().filter(|name| is_plugin_name(name)) {
            Some(name) if kept.len() < MAX_AGENT_DISABLED_PLUGINS => {
                kept.insert(name.to_string());
            }
            Some(_) => fields.invalid(
                &field,
                Some("the disabled plugin list holds too many names; this one was dropped and its plugin is enabled"),
            ),
            None => fields.invalid(
                &field,
                Some("a disabled plugin name that no plugin can carry was dropped"),
            ),
        }
    }
    read.mark(DISABLED_PLUGINS);
    kept
}
