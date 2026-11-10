// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/codex-mcp/src/agent_plugin_config.rs` (an Agent Plugins 1.0.0
// `mcp.json`: its `$schema`, its three server types and nothing else, a
// command that is a bare name or a contained `./` path, `${PLUGIN_ROOT}`
// and `${PLUGIN_DATA}` expanded and reserved, a contained `cwd`, an HTTPS
// URL unless on loopback, the client's own headers dropped) at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a field the schema does not define fails
// its server (the file's own unknown fields fail the file), checked against
// the schema's field lists rather than by `deny_unknown_fields`; paths are
// resolved and compared through `hanafubuki-agent-foundation::paths`, so a
// `./` command or a `cwd` must exist; a server becomes this crate's
// `McpServerConfig`.

use std::collections::BTreeMap;
use std::path::Path;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use serde::Deserialize;
use serde_json::{Map, Value};
use url::Host;

use super::{server_config, PluginMcpServerError, PluginMcpServers};
use crate::{McpOAuthPolicy, McpStdioConfig, McpStreamableHttpConfig, McpTransportConfig};

const SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";
const FILE_FIELDS: &[&str] = &["$schema", "mcpServers"];
const STDIO_FIELDS: &[&str] = &["type", "command", "args", "env", "cwd"];
const HTTP_FIELDS: &[&str] = &["type", "url", "headers"];
const ROOT: &str = "${PLUGIN_ROOT}";
const DATA: &str = "${PLUGIN_DATA}";
const CLIENT_OWNED_HEADERS: &[&str] = &[
    "accept",
    "authorization",
    "connection",
    "content-encoding",
    "content-length",
    "content-type",
    "host",
    "last-event-id",
    "mcp-protocol-version",
    "mcp-session-id",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "user-agent",
];

#[derive(Deserialize)]
struct StdioServer {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct HttpServer {
    url: String,
    headers: Option<BTreeMap<String, String>>,
}

/// The servers of an Agent Plugins package's `mcp.json`, `contents`. The
/// package is installed at `root`, with its data directory at `data`; both
/// exist. A file that does not follow the schema fails as a whole; a server
/// that does not, alone.
pub fn agent_plugin_mcp_servers(
    root: &Path,
    data: &Path,
    contents: &str,
) -> Result<PluginMcpServers, String> {
    let file = match serde_json::from_str::<Value>(contents) {
        Ok(Value::Object(file)) => file,
        Ok(_) => return Err("the file is not a JSON object".to_string()),
        Err(error) => return Err(error.to_string()),
    };
    if let Some(field) = unknown_field(&file, FILE_FIELDS) {
        return Err(format!("`{field}` is not an Agent Plugins MCP field"));
    }
    match file.get("$schema").and_then(Value::as_str) {
        Some(SCHEMA) => {}
        other => {
            return Err(format!(
                "unsupported Agent Plugins MCP schema {}; supported: `{SCHEMA}`",
                other.map_or_else(|| "(none)".to_string(), |schema| format!("`{schema}`"))
            ))
        }
    }
    let Some(Value::Object(servers)) = file.get("mcpServers") else {
        return Err("`mcpServers` must be an object".to_string());
    };
    let roots = Roots::resolve(root, data)?;
    let mut outcome = PluginMcpServers::default();
    for (name, value) in servers {
        match server(name, value, &roots) {
            Ok(server) => outcome.servers.push(server),
            Err(message) => outcome.errors.push(PluginMcpServerError {
                name: name.clone(),
                message,
            }),
        }
    }
    Ok(outcome)
}

fn server(name: &str, value: &Value, roots: &Roots) -> Result<crate::McpServerConfig, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "the server is not an object".to_string())?;
    let kind = object.get("type").and_then(Value::as_str);
    let fields = match kind {
        Some("stdio") => STDIO_FIELDS,
        Some("streamable-http" | "sse") => HTTP_FIELDS,
        _ => return Err("`type` must be `stdio`, `streamable-http` or `sse`".to_string()),
    };
    if let Some(field) = unknown_field(object, fields) {
        return Err(format!(
            "`{field}` is not a field of a `{}` server",
            kind.unwrap_or_default()
        ));
    }
    if let Some(field) = ["cwd", "headers"]
        .into_iter()
        .find(|field| object.get(*field).is_some_and(Value::is_null))
    {
        return Err(format!("`{field}` must use its declared type when present"));
    }
    let transport = match kind {
        Some("stdio") => {
            let server = StdioServer::deserialize(value).map_err(|error| error.to_string())?;
            stdio(server, roots)?
        }
        Some("streamable-http") => {
            let server = HttpServer::deserialize(value).map_err(|error| error.to_string())?;
            http(server)?
        }
        _ => return Err("the legacy SSE transport is not supported".to_string()),
    };
    let config = server_config(name, transport, None, None)?;
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn stdio(server: StdioServer, roots: &Roots) -> Result<McpTransportConfig, String> {
    let StdioServer {
        command,
        args,
        mut env,
        cwd,
    } = server;
    let bare = !command.is_empty()
        && !command.contains('/')
        && !command.contains('\\')
        && !Path::new(&command).has_root();
    let program = if bare {
        command
    } else if let Some(relative) = command.strip_prefix("./").filter(|tail| portable(tail)) {
        roots.contained(&roots.root, relative)?
    } else {
        return Err(
            "the command must be a bare executable name or a contained `./` path".to_string(),
        );
    };
    for reserved in ["PLUGIN_ROOT", "PLUGIN_DATA"] {
        if env.keys().any(|name| same_variable(name, reserved)) {
            return Err(format!(
                "`env` cannot override the reserved variable `{reserved}`"
            ));
        }
    }
    if cfg!(windows) {
        let mut upper = BTreeMap::new();
        for (name, value) in env {
            if upper.insert(name.to_ascii_uppercase(), value).is_some() {
                return Err(format!("the variable `{name}` is set twice"));
            }
        }
        env = upper;
    }
    let mut environment = env
        .into_iter()
        .map(|(name, value)| (name, roots.expand(&value)))
        .collect::<BTreeMap<_, _>>();
    environment.insert("PLUGIN_ROOT".to_string(), roots.root_text.clone());
    environment.insert("PLUGIN_DATA".to_string(), roots.data_text.clone());
    let cwd = cwd.as_deref().unwrap_or(ROOT);
    let cwd = if cwd == "./" || cwd == ROOT {
        roots.root.native().to_path_buf()
    } else if cwd == DATA {
        roots.data.native().to_path_buf()
    } else if let Some(relative) = cwd.strip_prefix("./").filter(|tail| portable(tail)) {
        roots.contained(&roots.root, relative)?.into()
    } else if let Some(relative) = cwd
        .strip_prefix(&format!("{ROOT}/"))
        .filter(|tail| portable(tail))
    {
        roots.contained(&roots.root, relative)?.into()
    } else if let Some(relative) = cwd
        .strip_prefix(&format!("{DATA}/"))
        .filter(|tail| portable(tail))
    {
        roots.contained(&roots.data, relative)?.into()
    } else {
        return Err(
            "`cwd` must be a contained `./`, `${PLUGIN_ROOT}` or `${PLUGIN_DATA}` path".to_string(),
        );
    };
    Ok(McpTransportConfig::Stdio(McpStdioConfig {
        program,
        argv: args.iter().map(|argument| roots.expand(argument)).collect(),
        cwd: Some(cwd),
        public_environment: environment,
        credential_environment: BTreeMap::new(),
    }))
}

fn http(server: HttpServer) -> Result<McpTransportConfig, String> {
    validate_url(&server.url)?;
    let mut headers = server.headers.unwrap_or_default();
    validate_headers(&headers)?;
    headers.retain(|name, _| {
        !CLIENT_OWNED_HEADERS
            .iter()
            .any(|owned| name.eq_ignore_ascii_case(owned))
    });
    Ok(McpTransportConfig::StreamableHttp(
        McpStreamableHttpConfig {
            url: server.url,
            public_headers: headers,
            credential_headers: BTreeMap::new(),
            oauth: McpOAuthPolicy::Disabled,
        },
    ))
}

fn validate_url(raw: &str) -> Result<(), String> {
    let url = url::Url::parse(raw).map_err(|error| format!("invalid URL `{raw}`: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("the URL must be absolute HTTP or HTTPS".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err("the URL must not carry user information or a fragment".to_string());
    }
    let loopback = match url.host() {
        Some(Host::Domain(host)) => host == "localhost",
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    if url.scheme() == "http" && !loopback {
        return Err("an endpoint off loopback must use HTTPS".to_string());
    }
    Ok(())
}

fn validate_headers(headers: &BTreeMap<String, String>) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for (name, value) in headers {
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(format!("the header `{name}` is set twice"));
        }
        let valid_name = !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte));
        if !valid_name {
            return Err(format!("invalid header name `{name}`"));
        }
        if value
            .bytes()
            .any(|byte| (byte < 32 && byte != b'\t') || byte == 127)
        {
            return Err(format!("invalid value for the header `{name}`"));
        }
    }
    Ok(())
}

/// The package and its data directory, resolved.
struct Roots {
    root: ResolvedPath,
    data: ResolvedPath,
    root_text: String,
    data_text: String,
}

impl Roots {
    fn resolve(root: &Path, data: &Path) -> Result<Self, String> {
        let resolve = |path: &Path| {
            ResolvedPath::resolve(path)
                .map_err(|error| format!("{} cannot be resolved: {error}", path.display()))
        };
        let (root, data) = (resolve(root)?, resolve(data)?);
        Ok(Self {
            root_text: root.native_text().to_string(),
            data_text: data.native_text().to_string(),
            root,
            data,
        })
    }

    /// `relative` under `base`, which must exist and stay inside it once
    /// its links are followed.
    fn contained(&self, base: &ResolvedPath, relative: &str) -> Result<String, String> {
        let path = ResolvedPath::resolve(base.native().join(relative))
            .map_err(|error| format!("`{relative}` cannot be resolved: {error}"))?;
        if !path.starts_with(base) {
            return Err(format!(
                "`{relative}` must stay within {}",
                base.native_text()
            ));
        }
        Ok(path.native_text().to_string())
    }

    fn expand(&self, value: &str) -> String {
        value
            .replace(ROOT, &self.root_text)
            .replace(DATA, &self.data_text)
    }
}

fn unknown_field<'a>(object: &'a Map<String, Value>, fields: &[&str]) -> Option<&'a str> {
    object
        .keys()
        .map(String::as_str)
        .find(|field| !fields.contains(field))
}

fn portable(relative: &str) -> bool {
    !relative.is_empty() && !relative.contains('\\')
}

fn same_variable(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}
