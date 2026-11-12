use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::compaction::decode_compaction;
use super::model_metadata::decode_model_metadata;
use super::packages::{decode_marketplaces, decode_plugins};
use super::{
    is_plugin_name, normalize_search_endpoint, validate_memory, validate_sandbox_workspace_write,
    AgentMemorySettings, AgentSandboxMode, AgentSandboxWorkspaceWrite, AgentSettings,
    AgentWebSearchProvider, AgentWindowsSettings, MAX_AGENT_DISABLED_PLUGINS,
};
use crate::fields::{read_member, read_object, FieldRead, Fields};
use crate::plugin_options::decode_plugin_options;

/// The settings `object` holds; a missing or invalid field takes its value
/// in `defaults` (the defaults, or the layer below in a layered
/// configuration). `layer` names the layer `object` is, which each plugin
/// option it sets remembers.
pub(crate) fn decode_settings(
    object: &Map<String, Value>,
    defaults: &AgentSettings,
    layer: &str,
    repairs: &mut Vec<String>,
) -> AgentSettings {
    let mut fields = Fields::new(object, repairs);
    let permission_mode = fields.read("permissionMode", defaults.permission_mode);
    let sandbox_mode = decode_sandbox_mode(&mut fields, defaults.sandbox_mode);
    let sandbox_workspace_write = read_object(
        &mut fields,
        "sandboxWorkspaceWrite",
        &defaults.sandbox_workspace_write,
        decode_sandbox_workspace_write,
    );
    let windows = read_object(&mut fields, "windows", &defaults.windows, decode_windows);
    let memory = read_object(&mut fields, "memory", &defaults.memory, decode_memory);
    let disabled_plugins = decode_disabled_plugins(&mut fields, &defaults.disabled_plugins);
    let searxng_base_url = fields.read_checked(
        "searxngBaseUrl",
        defaults.searxng_base_url.clone(),
        |value: Option<String>| value.as_deref().map(normalize_search_endpoint).transpose(),
    );
    let web_search_provider = decode_search_provider(
        &mut fields,
        defaults.web_search_provider,
        searxng_base_url.is_some(),
    );
    let web_fetch_use_jina_reader =
        fields.read("webFetchUseJinaReader", defaults.web_fetch_use_jina_reader);
    let web_fetch_resolve_through_proxy = fields.read(
        "webFetchResolveThroughProxy",
        defaults.web_fetch_resolve_through_proxy,
    );
    let compaction = read_object(
        &mut fields,
        "compaction",
        &defaults.compaction,
        decode_compaction,
    );
    let model_metadata = read_object(
        &mut fields,
        "modelMetadata",
        &defaults.model_metadata,
        decode_model_metadata,
    );
    let plugins = decode_plugins(&mut fields, &defaults.plugins);
    let marketplaces = decode_marketplaces(&mut fields, &defaults.marketplaces);
    let plugin_options = decode_plugin_options(&mut fields, &defaults.plugin_options, layer);
    AgentSettings {
        permission_mode,
        sandbox_mode,
        sandbox_workspace_write,
        windows,
        memory,
        disabled_plugins,
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

/// A mode that cannot be read is read-only, the most restrictive: the user
/// asked for some sandbox, and an unreadable one never grants more.
fn decode_sandbox_mode(
    fields: &mut Fields<'_>,
    default: Option<AgentSandboxMode>,
) -> Option<AgentSandboxMode> {
    match fields.read_quiet::<AgentSandboxMode>("sandboxMode", Ok) {
        FieldRead::Valid(mode) => Some(mode),
        FieldRead::Missing => default,
        FieldRead::Invalid => {
            fields.invalid(
                "sandboxMode",
                Some("the sandbox mode is not supported; commands run read-only until one is chosen again"),
            );
            Some(AgentSandboxMode::ReadOnly)
        }
    }
}

/// Each falls back on its own; a list holding a relative root is dropped
/// whole, which leaves fewer places writable, never more.
fn decode_sandbox_workspace_write(
    fields: &mut Fields<'_>,
    settings: &mut AgentSandboxWorkspaceWrite,
) {
    let validate = validate_sandbox_workspace_write;
    read_member(
        fields,
        settings,
        "writableRoots",
        |s| &mut s.writable_roots,
        validate,
    );
    read_member(
        fields,
        settings,
        "networkAccess",
        |s| &mut s.network_access,
        validate,
    );
}

/// An unreadable sandbox falls back to the layer below, else to none, which
/// gives no Windows sandbox, so never a weaker one.
fn decode_windows(fields: &mut Fields<'_>, settings: &mut AgentWindowsSettings) {
    read_member(fields, settings, "sandbox", |s| &mut s.sandbox, |_| Ok(()));
}

/// Each memory setting falls back on its own; its bounds are the ones the
/// whole object is validated against.
fn decode_memory(fields: &mut Fields<'_>, memory: &mut AgentMemorySettings) {
    let validate = validate_memory;
    read_member(fields, memory, "enabled", |m| &mut m.enabled, validate);
    read_member(
        fields,
        memory,
        "useMemories",
        |m| &mut m.use_memories,
        validate,
    );
    read_member(
        fields,
        memory,
        "generateMemories",
        |m| &mut m.generate_memories,
        validate,
    );
    read_member(
        fields,
        memory,
        "excludeExternalContext",
        |m| &mut m.exclude_external_context,
        validate,
    );
    read_member(
        fields,
        memory,
        "extractionModel",
        |m| &mut m.extraction_model,
        validate,
    );
    read_member(
        fields,
        memory,
        "consolidationModel",
        |m| &mut m.consolidation_model,
        validate,
    );
    read_member(
        fields,
        memory,
        "maxSourceAgeDays",
        |m| &mut m.max_source_age_days,
        validate,
    );
    read_member(
        fields,
        memory,
        "minimumIdleHours",
        |m| &mut m.minimum_idle_hours,
        validate,
    );
    read_member(
        fields,
        memory,
        "maxSessionsPerPass",
        |m| &mut m.max_sessions_per_pass,
        validate,
    );
    read_member(
        fields,
        memory,
        "maxStage1Outputs",
        |m| &mut m.max_stage1_outputs,
        validate,
    );
    read_member(
        fields,
        memory,
        "maxUnusedDays",
        |m| &mut m.max_unused_days,
        validate,
    );
}

/// A name that cannot be one is dropped alone: losing the whole set would
/// enable what the user turned off.
fn decode_disabled_plugins(
    fields: &mut Fields<'_>,
    default: &BTreeSet<String>,
) -> BTreeSet<String> {
    let Some(raw) = fields.raw("disabledPlugins") else {
        return default.clone();
    };
    let Some(items) = raw.as_array() else {
        fields.invalid(
            "disabledPlugins",
            Some("the disabled plugin list is unreadable; every plugin is enabled until plugins are turned off again"),
        );
        return BTreeSet::new();
    };
    let mut kept = BTreeSet::new();
    for (index, value) in items.iter().enumerate() {
        let field = format!("disabledPlugins[{index}]");
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
    kept
}

/// A search endpoint is never accepted with credentials, a query, or a
/// fragment. Without a usable endpoint the provider cannot stay SearXNG,
/// and falls back to the automatic one.
fn decode_search_provider(
    fields: &mut Fields<'_>,
    default: AgentWebSearchProvider,
    has_endpoint: bool,
) -> AgentWebSearchProvider {
    match fields.read_quiet::<AgentWebSearchProvider>("webSearchProvider", Ok) {
        FieldRead::Valid(AgentWebSearchProvider::Searxng) if !has_endpoint => {
            fields.invalid(
                "webSearchProvider",
                Some("SearXNG has no usable endpoint; Agent web search uses the automatic provider until an endpoint is configured again"),
            );
            AgentWebSearchProvider::Auto
        }
        FieldRead::Valid(provider) => provider,
        FieldRead::Missing => default,
        FieldRead::Invalid => {
            fields.invalid(
                "webSearchProvider",
                Some("the web search provider is not supported; Agent web search uses the automatic provider until one is chosen again"),
            );
            default
        }
    }
}
