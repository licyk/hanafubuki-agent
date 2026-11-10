// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/codex-mcp/src/plugin_config.rs` (a plugin's `.mcp.json` in its
// two shapes, Claude Code's keys normalised, a relative `cwd` joined to the
// plugin's root, one bad server never costing its siblings) and the server
// fields of `codex-rs/config/src/mcp_types.rs` at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a server becomes this crate's
// `McpServerConfig` with the defaults a server the user adds starts with;
// Claude Code's `headers` and its `${CLAUDE_PLUGIN_ROOT}` and
// `${CLAUDE_PLUGIN_DATA}` (and `${PLUGIN_ROOT}`, `${PLUGIN_DATA}`) are
// read, and a stdio server runs with those four variables set; a relative
// command path is joined to the root; bearer-token and environment headers
// are refused, there being no credential to read them from; `oauth` asks
// for an optional login; executor-owned plugins are not ported.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    McpOAuthPolicy, McpServerConfig, McpServerDefaults, McpServerId, McpStdioConfig,
    McpStreamableHttpConfig, McpTransportConfig,
};

mod agent_plugins;

pub use agent_plugins::agent_plugin_mcp_servers;

/// The MCP servers a package declares: each one that can run, and why each
/// other one cannot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginMcpServers {
    pub servers: Vec<McpServerConfig>,
    pub errors: Vec<PluginMcpServerError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginMcpServerError {
    pub name: String,
    pub message: String,
}

/// The servers of a Codex, Claude Code or Cursor package's `.mcp.json` (or
/// its manifest's inline `mcpServers`), `document`: either
/// `{"mcpServers": {...}}` or the bare map of servers by name. `root` is
/// the installed package, `data` its own data directory.
pub fn plugin_mcp_servers(
    root: &Path,
    data: &Path,
    document: &Map<String, Value>,
) -> PluginMcpServers {
    let servers = match document.get("mcpServers") {
        Some(Value::Object(servers)) => servers,
        _ => document,
    };
    let mut outcome = PluginMcpServers::default();
    for (name, value) in servers {
        match legacy_server(name, value, root, data) {
            Ok(server) => outcome.servers.push(server),
            Err(message) => outcome.errors.push(PluginMcpServerError {
                name: name.clone(),
                message,
            }),
        }
    }
    outcome
}

/// A server as a Codex or Claude Code plugin writes it. Unknown fields are
/// ignored; a field of the wrong type fails the server.
#[derive(Deserialize)]
struct LegacyServer {
    command: Option<String>,
    args: Option<Vec<String>>,
    env: Option<BTreeMap<String, String>>,
    cwd: Option<String>,
    url: Option<String>,
    #[serde(alias = "headers")]
    http_headers: Option<BTreeMap<String, String>>,
    bearer_token_env_var: Option<String>,
    env_http_headers: Option<BTreeMap<String, String>>,
    enabled: Option<bool>,
    required: Option<bool>,
    startup_timeout_sec: Option<f64>,
    startup_timeout_ms: Option<u64>,
    tool_timeout_sec: Option<f64>,
    enabled_tools: Option<Vec<String>>,
    disabled_tools: Option<Vec<String>>,
    oauth: Option<Value>,
}

fn legacy_server(
    name: &str,
    value: &Value,
    root: &Path,
    data: &Path,
) -> Result<McpServerConfig, String> {
    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| "the server is not an object".to_string())?;
    if let Some(Value::String(kind)) = object.remove("type") {
        if !matches!(
            kind.as_str(),
            "http" | "streamable_http" | "streamable-http" | "stdio"
        ) {
            log::warn!("plugin MCP server {name} names an unknown transport type `{kind}`");
        }
    }
    let server =
        LegacyServer::deserialize(Value::Object(object)).map_err(|error| error.to_string())?;
    let placeholders = Placeholders::new(root, data);
    let transport = if let Some(command) = server.command {
        if server.url.is_some() {
            return Err("a server has a `command` or a `url`, not both".to_string());
        }
        let command = placeholders.expand(&command);
        let program = if command.contains('/') || command.contains('\\') {
            let path = Path::new(&command);
            if path.is_absolute() {
                command
            } else {
                root.join(path).display().to_string()
            }
        } else {
            command
        };
        let cwd = server.cwd.map(|cwd| {
            let cwd = placeholders.expand(&cwd);
            if Path::new(&cwd).is_absolute() {
                cwd.into()
            } else {
                root.join(cwd)
            }
        });
        let mut environment = placeholders.environment();
        environment.extend(
            server
                .env
                .unwrap_or_default()
                .into_iter()
                .map(|(name, value)| (name, placeholders.expand(&value))),
        );
        McpTransportConfig::Stdio(McpStdioConfig {
            program,
            argv: server
                .args
                .unwrap_or_default()
                .iter()
                .map(|argument| placeholders.expand(argument))
                .collect(),
            cwd,
            public_environment: environment,
            credential_environment: BTreeMap::new(),
        })
    } else if let Some(url) = server.url {
        if server.bearer_token_env_var.is_some() || server.env_http_headers.is_some() {
            return Err(
                "`bearer_token_env_var` and `env_http_headers` are not supported for a plugin's server"
                    .to_string(),
            );
        }
        McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
            url,
            public_headers: server.http_headers.unwrap_or_default(),
            credential_headers: BTreeMap::new(),
            oauth: if server.oauth.is_some() {
                McpOAuthPolicy::Optional
            } else {
                McpOAuthPolicy::Disabled
            },
        })
    } else {
        return Err("the server has neither a `command` nor a `url`".to_string());
    };
    let startup = match (server.startup_timeout_sec, server.startup_timeout_ms) {
        (Some(seconds), _) => Some(whole_seconds(seconds)?),
        (None, Some(milliseconds)) => Some(milliseconds.div_ceil(1000).max(1)),
        (None, None) => None,
    };
    let call = server.tool_timeout_sec.map(whole_seconds).transpose()?;
    let mut config = server_config(name, transport, startup, call)?;
    config.enabled = server.enabled.unwrap_or(true);
    config.required = server.required.unwrap_or(false);
    config.enabled_tools = server
        .enabled_tools
        .unwrap_or_default()
        .into_iter()
        .collect();
    config.disabled_tools = server
        .disabled_tools
        .unwrap_or_default()
        .into_iter()
        .collect();
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

/// `name` as a server that runs over `transport`, with the defaults of a
/// server the user adds, and the startup and call timeouts given.
fn server_config(
    name: &str,
    transport: McpTransportConfig,
    startup_seconds: Option<u64>,
    call_seconds: Option<u64>,
) -> Result<McpServerConfig, String> {
    let id = McpServerId::parse(name).map_err(|_| {
        "the server name must be 1 to 128 letters, digits, `-`, `_` or `.`".to_string()
    })?;
    let defaults = McpServerDefaults::default();
    Ok(McpServerConfig {
        id,
        enabled: defaults.enabled,
        required: defaults.required,
        protocol_mode: defaults.protocol_mode,
        transport,
        connect_timeout_seconds: startup_seconds.unwrap_or(defaults.connect_timeout_seconds),
        initialize_timeout_seconds: startup_seconds.unwrap_or(defaults.initialize_timeout_seconds),
        list_timeout_seconds: defaults.list_timeout_seconds,
        call_timeout_seconds: call_seconds.unwrap_or(defaults.call_timeout_seconds),
        shutdown_timeout_seconds: defaults.shutdown_timeout_seconds,
        maximum_concurrent_calls: defaults.maximum_concurrent_calls,
        tool_maximum_concurrent_calls: BTreeMap::new(),
        enabled_tools: BTreeSet::new(),
        disabled_tools: BTreeSet::new(),
        approval_policy: defaults.approval_policy,
        tool_approval_policies: BTreeMap::new(),
        exposure_policy: defaults.exposure_policy,
        tool_exposure_policies: BTreeMap::new(),
    })
}

fn whole_seconds(seconds: f64) -> Result<u64, String> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(format!("{seconds} is not a timeout"));
    }
    Ok((seconds.ceil() as u64).max(1))
}

/// The package's root and data directory, as a server's strings name them
/// and as its environment carries them.
struct Placeholders {
    root: String,
    data: String,
}

impl Placeholders {
    fn new(root: &Path, data: &Path) -> Self {
        Self {
            root: root.display().to_string(),
            data: data.display().to_string(),
        }
    }

    /// `value` with `${PLUGIN_ROOT}`, `${PLUGIN_DATA}` and Claude Code's
    /// `${CLAUDE_PLUGIN_ROOT}` and `${CLAUDE_PLUGIN_DATA}` replaced.
    fn expand(&self, value: &str) -> String {
        value
            .replace("${CLAUDE_PLUGIN_ROOT}", &self.root)
            .replace("${CLAUDE_PLUGIN_DATA}", &self.data)
            .replace("${PLUGIN_ROOT}", &self.root)
            .replace("${PLUGIN_DATA}", &self.data)
    }

    fn environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("PLUGIN_ROOT".to_string(), self.root.clone()),
            ("CLAUDE_PLUGIN_ROOT".to_string(), self.root.clone()),
            ("PLUGIN_DATA".to_string(), self.data.clone()),
            ("CLAUDE_PLUGIN_DATA".to_string(), self.data.clone()),
        ])
    }
}

#[cfg(test)]
mod tests;
