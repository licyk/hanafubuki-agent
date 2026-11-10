//! `[plugins]` and `[marketplaces]`, read over the layer below entry by
//! entry, as Codex merges these tables across its layers: an entry a layer
//! names changes only the fields it sets. A field that cannot be used keeps
//! the value below and is a repair. An entry that cannot be one (a key that
//! is not a plugin id or a marketplace name, a value that is not a table, a
//! marketplace without its source) is left out and reported as an unknown
//! key, so the file keeps it as written.

use std::collections::BTreeMap;

use hanafubuki_agent_packages::{
    validate_marketplace_name, MarketplaceRecord, MarketplaceSourceType, PluginDefaults, PluginId,
    PluginMcpServerSettings, PluginSettings, PluginsSettings,
};
use serde_json::{Map, Value};

use crate::fields::{FieldRead, Fields};

const DEFAULT_KEY: &str = "_default";

pub(super) fn decode_plugins(fields: &mut Fields<'_>, below: &PluginsSettings) -> PluginsSettings {
    let mut plugins = below.clone();
    let Some(object) = table(fields, "plugins") else {
        return plugins;
    };
    let path = fields.path("plugins");
    for (key, value) in object {
        let Some(entry) = value.as_object() else {
            continue;
        };
        let mut entry_fields = Fields::nested(entry, format!("{path}.{key}"), fields.repairs());
        if key == DEFAULT_KEY {
            let enabled = below.default.as_ref().is_none_or(|default| default.enabled);
            plugins.default = Some(PluginDefaults {
                enabled: entry_fields.read("enabled", enabled),
            });
        } else if PluginId::parse(key).is_ok() {
            let below = below.packages.get(key).cloned().unwrap_or_default();
            plugins
                .packages
                .insert(key.clone(), decode_plugin(&mut entry_fields, below));
        }
    }
    plugins
}

fn decode_plugin(fields: &mut Fields<'_>, below: PluginSettings) -> PluginSettings {
    let enabled = fields.read("enabled", below.enabled);
    let mut mcp_servers = below.mcp_servers;
    if let Some(servers) = table(fields, "mcpServers") {
        let path = fields.path("mcpServers");
        for (name, value) in servers {
            let Some(server) = value.as_object() else {
                continue;
            };
            let below = mcp_servers.get(name).cloned().unwrap_or_default();
            let mut server_fields =
                Fields::nested(server, format!("{path}.{name}"), fields.repairs());
            mcp_servers.insert(name.clone(), decode_mcp_server(&mut server_fields, below));
        }
    }
    PluginSettings {
        enabled,
        mcp_servers,
    }
}

fn decode_mcp_server(
    fields: &mut Fields<'_>,
    below: PluginMcpServerSettings,
) -> PluginMcpServerSettings {
    PluginMcpServerSettings {
        enabled: fields.read("enabled", below.enabled),
        enabled_tools: fields.read("enabledTools", below.enabled_tools),
        disabled_tools: fields.read("disabledTools", below.disabled_tools),
    }
}

pub(super) fn decode_marketplaces(
    fields: &mut Fields<'_>,
    below: &BTreeMap<String, MarketplaceRecord>,
) -> BTreeMap<String, MarketplaceRecord> {
    let mut marketplaces = below.clone();
    let Some(object) = table(fields, "marketplaces") else {
        return marketplaces;
    };
    let path = fields.path("marketplaces");
    for (name, value) in object {
        let Some(entry) = value.as_object() else {
            continue;
        };
        if validate_marketplace_name(name).is_err() {
            continue;
        }
        let mut entry_fields = Fields::nested(entry, format!("{path}.{name}"), fields.repairs());
        if let Some(record) = decode_marketplace(&mut entry_fields, below.get(name)) {
            marketplaces.insert(name.clone(), record);
        }
    }
    marketplaces
}

fn decode_marketplace(
    fields: &mut Fields<'_>,
    below: Option<&MarketplaceRecord>,
) -> Option<MarketplaceRecord> {
    let source_type = match fields.read_quiet::<MarketplaceSourceType>("sourceType", Ok) {
        FieldRead::Valid(source_type) => Some(source_type),
        FieldRead::Missing => below.map(|record| record.source_type),
        FieldRead::Invalid => {
            fields.invalid("sourceType", None);
            below.map(|record| record.source_type)
        }
    };
    let non_empty = |source: String| {
        if source.trim().is_empty() {
            Err("empty".to_string())
        } else {
            Ok(source)
        }
    };
    let source = match fields.read_quiet::<String>("source", non_empty) {
        FieldRead::Valid(source) => Some(source),
        FieldRead::Missing => below.map(|record| record.source.clone()),
        FieldRead::Invalid => {
            fields.invalid("source", None);
            below.map(|record| record.source.clone())
        }
    };
    Some(MarketplaceRecord {
        source_type: source_type?,
        source: source?,
        ref_name: fields.read("ref", below.and_then(|record| record.ref_name.clone())),
        sparse_paths: fields.read(
            "sparsePaths",
            below.and_then(|record| record.sparse_paths.clone()),
        ),
        last_updated: fields.read(
            "lastUpdated",
            below.and_then(|record| record.last_updated.clone()),
        ),
        last_revision: fields.read(
            "lastRevision",
            below.and_then(|record| record.last_revision.clone()),
        ),
        enabled: fields.read("enabled", below.and_then(|record| record.enabled)),
    })
}

/// The table at `key`; another value is a repair, and the layer below's
/// table is kept.
fn table<'a>(fields: &mut Fields<'a>, key: &str) -> Option<&'a Map<String, Value>> {
    match fields.raw(key)? {
        Value::Object(object) => Some(object),
        _ => {
            fields.invalid(key, None);
            None
        }
    }
}

#[cfg(test)]
mod tests;
