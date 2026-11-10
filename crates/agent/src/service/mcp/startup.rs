//! Connecting the MCP servers without holding anything up.
//!
//! The Agent's start connects every enabled server in the background
//! ([`AgentMcpApplication::spawn_start`]), so the Agent runs without waiting
//! for them; a proxy change reconnects the Streamable HTTP servers the same
//! way. Servers connect concurrently, at most [`MCP_CONNECT_CONCURRENCY`] at
//! once, each bounded by its own connect and initialize timeouts, and each
//! server's MCP status says how its connection went. Every connect the
//! application makes observes the MCP lifetime token, which shutdown cancels
//! first ([`AgentMcpApplication::stop_connecting`]): a connect under way then
//! stops and disposes the process it spawned.

use std::sync::Arc;
use std::time::Instant;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tokio::task::{JoinHandle, JoinSet};

use super::AgentMcpApplication;

/// How many MCP servers connect at once.
pub(crate) const MCP_CONNECT_CONCURRENCY: usize = 4;

/// The MCP side's lifetime: cancelled by shutdown, and the background start
/// it may still be running.
#[derive(Default)]
pub(crate) struct McpLifetime {
    cancellation: CancellationToken,
    start: Mutex<Option<JoinHandle<Option<()>>>>,
}

impl McpLifetime {
    /// A token every connect observes; cancelled at shutdown.
    pub(crate) fn token(&self) -> CancellationToken {
        self.cancellation.child()
    }
}

impl AgentMcpApplication {
    /// Connects every enabled server (the user's, then the plugins') in
    /// the background, once per Agent instance. An optional server that fails is a warning, a required one
    /// an error; either way the failure is that server's MCP status, and the
    /// Agent runs without it.
    pub(crate) fn spawn_start(
        &self,
        kernel_faults: &Arc<dyn hanafubuki_agent_kernel::AgentKernelFaultSink>,
    ) {
        let mut start = self.lifetime.start.lock();
        if start.is_some() {
            log::warn!("the MCP servers are already starting; a second start is ignored");
            return;
        }
        let application = self.clone();
        *start = Some(crate::kernel_tasks::spawn_kernel_task(
            kernel_faults,
            "MCP start",
            async move {
                match application.start_servers().await {
                    Ok(warnings) => {
                        for warning in warnings {
                            log::warn!("{warning}");
                        }
                    }
                    Err(error) => {
                        log::error!("the Agent runs without a required MCP server: {error}")
                    }
                }
            },
        ));
    }

    /// Connects every enabled server, concurrently, and returns once each
    /// connected or failed. An optional server that fails is a warning; a
    /// required one is an error, returned after every server was tried.
    pub(crate) async fn start_servers(&self) -> Result<Vec<String>, String> {
        let settings = Arc::new(self.load_settings().await?);
        let servers = settings
            .servers
            .iter()
            .cloned()
            .chain(
                self.plugin_servers()
                    .into_iter()
                    .map(|plugin| plugin.server),
            )
            .filter(|server| server.enabled)
            .collect();
        let mut warnings = Vec::new();
        let mut required_failures = Vec::new();
        for (server, outcome) in self.connect_concurrently(servers, settings).await {
            let Err(error) = outcome else {
                continue;
            };
            let message = format!(
                "{} MCP server {} did not start: {error}",
                if server.required {
                    "required"
                } else {
                    "optional"
                },
                server.id.as_str()
            );
            if server.required {
                required_failures.push(message);
            } else {
                warnings.push(message);
            }
        }
        if required_failures.is_empty() {
            Ok(warnings)
        } else {
            Err(required_failures
                .into_iter()
                .chain(warnings)
                .collect::<Vec<_>>()
                .join("; "))
        }
    }

    /// Connects `servers`, at most [`MCP_CONNECT_CONCURRENCY`] at once, and
    /// returns each one's outcome in the order given. A connect that panics
    /// is that server's failure.
    pub(crate) async fn connect_concurrently(
        &self,
        servers: Vec<hanafubuki_agent_mcp::McpServerConfig>,
        settings: Arc<hanafubuki_agent_mcp::McpSettings>,
    ) -> Vec<(hanafubuki_agent_mcp::McpServerConfig, Result<(), String>)> {
        let permits = Arc::new(Semaphore::new(MCP_CONNECT_CONCURRENCY));
        let mut connects = JoinSet::new();
        for (index, server) in servers.iter().cloned().enumerate() {
            let application = self.clone();
            let settings = settings.clone();
            let permits = permits.clone();
            connects.spawn(async move {
                let Ok(_permit) = permits.acquire_owned().await else {
                    return (index, Err("MCP connect was not admitted".to_string()));
                };
                (
                    index,
                    application.connect_mcp_server(&server, &settings).await,
                )
            });
        }
        let mut outcomes: Vec<Option<Result<(), String>>> = vec![None; servers.len()];
        while let Some(joined) = connects.join_next().await {
            match joined {
                Ok((index, outcome)) => outcomes[index] = Some(outcome),
                Err(error) => log::error!("an MCP connect did not complete: {error}"),
            }
        }
        servers
            .into_iter()
            .zip(outcomes)
            .map(|(server, outcome)| {
                let outcome =
                    outcome.unwrap_or_else(|| Err("the MCP connect did not complete".to_string()));
                (server, outcome)
            })
            .collect()
    }

    /// Cancels every connect under way and waits, until `deadline`, for the
    /// background start to end; each cancelled connect disposes the process
    /// it spawned. The manager's own shutdown closes what is connected.
    pub(crate) async fn stop_connecting(&self, deadline: Instant) {
        self.lifetime.cancellation.cancel();
        let Some(start) = self.lifetime.start.lock().take() else {
            return;
        };
        if tokio::time::timeout_at(deadline.into(), start)
            .await
            .is_err()
        {
            log::warn!("the MCP start did not stop before the shutdown deadline");
        }
    }
}
