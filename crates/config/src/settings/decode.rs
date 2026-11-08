use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::{
    is_plugin_name, normalize_search_endpoint, validate_memory, AgentMemorySettings, AgentSettings,
    AgentWebSearchProvider, MAX_AGENT_DISABLED_PLUGINS,
};
use crate::fields::{read_member, read_object, FieldRead, Fields};

pub(crate) fn decode_settings(
    object: &Map<String, Value>,
    repairs: &mut Vec<String>,
) -> AgentSettings {
    let defaults = AgentSettings::default();
    let mut fields = Fields::new(object, repairs);
    let permission_mode = fields.read("permissionMode", defaults.permission_mode);
    let memory = read_object(&mut fields, "memory", &defaults.memory, decode_memory);
    let disabled_plugins = decode_disabled_plugins(&mut fields);
    let searxng_base_url = fields.read_checked(
        "searxngBaseUrl",
        defaults.searxng_base_url.clone(),
        |value: Option<String>| value.as_deref().map(normalize_search_endpoint).transpose(),
    );
    let web_search_provider = decode_search_provider(&mut fields, searxng_base_url.is_some());
    let web_fetch_use_jina_reader =
        fields.read("webFetchUseJinaReader", defaults.web_fetch_use_jina_reader);
    AgentSettings {
        permission_mode,
        memory,
        disabled_plugins,
        web_search_provider,
        searxng_base_url,
        web_fetch_use_jina_reader,
    }
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
fn decode_disabled_plugins(fields: &mut Fields<'_>) -> BTreeSet<String> {
    let Some(raw) = fields.raw("disabledPlugins") else {
        return BTreeSet::new();
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
/// fragment. Without a usable endpoint the provider cannot stay SearXNG.
fn decode_search_provider(fields: &mut Fields<'_>, has_endpoint: bool) -> AgentWebSearchProvider {
    let default = AgentWebSearchProvider::Auto;
    match fields.read_quiet::<AgentWebSearchProvider>("webSearchProvider", Ok) {
        FieldRead::Valid(AgentWebSearchProvider::Searxng) if !has_endpoint => {
            fields.invalid(
                "webSearchProvider",
                Some("SearXNG has no usable endpoint; Agent web search uses the automatic provider until an endpoint is configured again"),
            );
            default
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
