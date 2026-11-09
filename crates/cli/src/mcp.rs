//! `mcp list|add|remove|connect|login`. The list reads `mcp.toml`; the
//! changes, the connect and the login go through the running Agent's MCP
//! API, which connects a server it is told about.

use std::collections::BTreeMap;
use std::io::Write;

use hanafubuki_agent::{
    AgentRuntimeHost, McpApplicationSnapshot, McpConnectionPhase, McpOAuthPolicy, McpServerConfig,
    McpServerDeleteRequest, McpServerId, McpServerWriteRequest, McpStdioConfig,
    McpStreamableHttpConfig, McpTransportConfig,
};

use crate::agent::{block_on, with_agent, Standalone};
use crate::cli::{GlobalArgs, McpCommand, OAuth};
use crate::exit::Failure;

pub(crate) fn run(global: &GlobalArgs, command: McpCommand) -> Result<(), Failure> {
    match command {
        McpCommand::List => {
            let standalone = Standalone::open(global, AgentRuntimeHost::default())?;
            let settings = standalone.runtime.config().mcp().load();
            standalone.shutdown();
            let settings = settings.map_err(|error| Failure::new(error.to_string()))?;
            for server in &settings.servers {
                println!("{}", describe(server));
            }
            for key in settings.credentials.keys() {
                println!("credential {} (set)", key.as_str());
            }
            Ok(())
        }
        McpCommand::Add {
            id,
            url,
            oauth,
            environment,
            disabled,
            command,
        } => {
            let id = server_id(&id)?;
            let transport = transport(url, oauth, environment, command)?;
            with_agent(global, |agent| {
                let snapshot = block_on(agent, move |agent| async move {
                    let current = agent.mcp_snapshot().await?;
                    let defaults = current.server_defaults;
                    let server = McpServerConfig {
                        id,
                        enabled: !disabled,
                        required: defaults.required,
                        protocol_mode: defaults.protocol_mode,
                        transport,
                        connect_timeout_seconds: defaults.connect_timeout_seconds,
                        initialize_timeout_seconds: defaults.initialize_timeout_seconds,
                        list_timeout_seconds: defaults.list_timeout_seconds,
                        call_timeout_seconds: defaults.call_timeout_seconds,
                        shutdown_timeout_seconds: defaults.shutdown_timeout_seconds,
                        maximum_concurrent_calls: defaults.maximum_concurrent_calls,
                        tool_maximum_concurrent_calls: BTreeMap::new(),
                        enabled_tools: Default::default(),
                        disabled_tools: Default::default(),
                        approval_policy: defaults.approval_policy,
                        tool_approval_policies: BTreeMap::new(),
                        exposure_policy: defaults.exposure_policy,
                        tool_exposure_policies: BTreeMap::new(),
                    };
                    agent
                        .mcp_write_server(McpServerWriteRequest {
                            expected_revision: current.settings.revision,
                            server,
                        })
                        .await
                })?;
                print_status(&snapshot);
                Ok(())
            })
        }
        McpCommand::Remove { id } => {
            let server_id = server_id(&id)?;
            with_agent(global, |agent| {
                block_on(agent, move |agent| async move {
                    let current = agent.mcp_snapshot().await?;
                    agent
                        .mcp_delete_server(McpServerDeleteRequest {
                            expected_revision: current.settings.revision,
                            server_id,
                        })
                        .await
                })
                .map(|_| ())
            })
        }
        McpCommand::Connect { id } => {
            let server_id = server_id(&id)?;
            with_agent(global, |agent| {
                let connecting = server_id.clone();
                let snapshot = block_on(agent, move |agent| async move {
                    agent.mcp_connect(connecting).await
                })?;
                let status = snapshot
                    .servers
                    .iter()
                    .find(|status| status.server_id == server_id)
                    .ok_or_else(|| Failure::new(format!("no MCP server {id}")))?;
                println!(
                    "{id}: {} ({} tools)",
                    phase(status.phase),
                    status.tool_count
                );
                match (&status.phase, &status.last_error) {
                    (McpConnectionPhase::Ready, _) => Ok(()),
                    (_, Some(error)) => Err(Failure::new(error.clone())),
                    (_, None) => Err(Failure::new(format!("{id} is not connected"))),
                }
            })
        }
        // No browser is opened: the URL is printed for the user to open, on
        // this machine since the authorization comes back to a local port.
        McpCommand::Login { id } => {
            let server_id = server_id(&id)?;
            with_agent(global, |agent| {
                let snapshot = block_on(agent, move |agent| async move {
                    agent
                        .mcp_oauth_login(
                            server_id,
                            |url| {
                                println!("Open this URL in a browser on this machine to log in:");
                                println!("{url}");
                                let _ = std::io::stdout().flush();
                            },
                            std::future::pending(),
                        )
                        .await
                })?;
                println!("{id}: logged in");
                print_status(&snapshot);
                Ok(())
            })
        }
    }
}

fn server_id(id: &str) -> Result<McpServerId, Failure> {
    McpServerId::parse(id).map_err(|_| {
        Failure::usage(format!(
            "`{id}` is not an MCP server id (letters, digits, `-`, `_` and `.`)"
        ))
    })
}

fn transport(
    url: Option<String>,
    oauth: Option<OAuth>,
    environment: Vec<String>,
    command: Vec<String>,
) -> Result<McpTransportConfig, Failure> {
    if let Some(url) = url {
        if !environment.is_empty() {
            return Err(Failure::usage("--env is for a stdio server"));
        }
        return Ok(McpTransportConfig::StreamableHttp(
            McpStreamableHttpConfig {
                url,
                public_headers: BTreeMap::new(),
                credential_headers: BTreeMap::new(),
                oauth: match oauth {
                    None | Some(OAuth::Disabled) => McpOAuthPolicy::Disabled,
                    Some(OAuth::Optional) => McpOAuthPolicy::Optional,
                    Some(OAuth::Required) => McpOAuthPolicy::Required,
                },
            },
        ));
    }
    if oauth.is_some() {
        return Err(Failure::usage("--oauth is for a Streamable HTTP server"));
    }
    let mut command = command.into_iter();
    let program = command
        .next()
        .ok_or_else(|| Failure::usage("give --url URL, or the program after `--`"))?;
    let public_environment = environment
        .iter()
        .map(|entry| {
            entry
                .split_once('=')
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .ok_or_else(|| Failure::usage(format!("--env {entry} is not KEY=VALUE")))
        })
        .collect::<Result<_, _>>()?;
    Ok(McpTransportConfig::Stdio(McpStdioConfig {
        program,
        argv: command.collect(),
        cwd: None,
        public_environment,
        credential_environment: BTreeMap::new(),
    }))
}

/// A server without its environment's and headers' values, which may be
/// secrets even when they are not credentials.
fn describe(server: &McpServerConfig) -> String {
    let state = if server.enabled { "" } else { " (disabled)" };
    let target = match &server.transport {
        McpTransportConfig::Stdio(stdio) => {
            let mut target = format!("stdio {}", stdio.program);
            for argument in &stdio.argv {
                target.push(' ');
                target.push_str(argument);
            }
            let names = stdio
                .public_environment
                .keys()
                .chain(stdio.credential_environment.keys())
                .map(String::as_str)
                .collect::<Vec<_>>();
            if !names.is_empty() {
                target.push_str(&format!("  env: {}", names.join(", ")));
            }
            target
        }
        McpTransportConfig::StreamableHttp(http) => {
            let mut target = format!("http {}", http.url);
            let names = http
                .public_headers
                .keys()
                .chain(http.credential_headers.keys())
                .map(String::as_str)
                .collect::<Vec<_>>();
            if !names.is_empty() {
                target.push_str(&format!("  headers: {}", names.join(", ")));
            }
            target
        }
    };
    format!("{}{state}  {target}", server.id.as_str())
}

fn print_status(snapshot: &McpApplicationSnapshot) {
    for status in &snapshot.servers {
        let error = status
            .last_error
            .as_deref()
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
        println!(
            "{}: {}{error}",
            status.server_id.as_str(),
            phase(status.phase)
        );
    }
}

/// A connection phase as it is serialized (`ready`).
pub(crate) fn phase(phase: McpConnectionPhase) -> String {
    serde_json::to_value(phase)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}
