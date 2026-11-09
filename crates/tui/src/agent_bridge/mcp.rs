//! `/mcp` onto the facade's MCP calls: the report (`/mcp verbose`), and
//! the manager's reading, writing, connecting and logging in.
//!
//! A server is written before its secrets: the credential calls take no
//! revision, so the server's write is the one the screen's revision gates,
//! and a credential it no longer references can be removed after it.

use std::fmt;

use hanafubuki_agent::{
    AgentHandle, McpApplicationSnapshot, McpCredentialKey, McpServerConfig, McpServerDeleteRequest,
    McpServerId, McpServerWriteRequest, McpTransportConfig,
};
use tokio::sync::oneshot;

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app_event::AppEvent;

/// What `/mcp` asks of the Agent.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum McpCall {
    /// The servers and their tools, written into the transcript.
    Report,
    Load,
    Write {
        expected_revision: u64,
        server: Box<McpServerConfig>,
        /// The secrets to set, each under its key.
        secrets: Vec<(McpCredentialKey, String)>,
        /// The keys the server referenced before, removed once no server
        /// references them.
        dropped: Vec<McpCredentialKey>,
    },
    Delete {
        expected_revision: u64,
        server_id: McpServerId,
        dropped: Vec<McpCredentialKey>,
    },
    Connect(McpServerId),
    Refresh(McpServerId),
    Login(McpServerId),
    /// Ends the login that waits for the browser.
    CancelLogin,
    Logout(McpServerId),
}

/// What the Agent answered.
pub(crate) enum McpReply {
    Report(McpApplicationSnapshot),
    Loaded(McpApplicationSnapshot),
    Changed(McpApplicationSnapshot),
    /// The login waits for the browser at this address.
    LoginUrl {
        server_id: McpServerId,
        url: String,
    },
    LoggedIn {
        server_id: McpServerId,
        snapshot: McpApplicationSnapshot,
    },
    /// The call failed; `reloaded` is the settings when their revision
    /// moved since the screen read them.
    Failed {
        error: String,
        reloaded: Option<McpApplicationSnapshot>,
    },
}

/// Without the servers and the secrets.
impl fmt::Debug for McpCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Report => formatter.write_str("Report"),
            Self::Load => formatter.write_str("Load"),
            Self::Write { server, .. } => write!(formatter, "Write({})", server.id.as_str()),
            Self::Delete { server_id, .. } => write!(formatter, "Delete({})", server_id.as_str()),
            Self::Connect(id) => write!(formatter, "Connect({})", id.as_str()),
            Self::Refresh(id) => write!(formatter, "Refresh({})", id.as_str()),
            Self::Login(id) => write!(formatter, "Login({})", id.as_str()),
            Self::CancelLogin => formatter.write_str("CancelLogin"),
            Self::Logout(id) => write!(formatter, "Logout({})", id.as_str()),
        }
    }
}

impl fmt::Debug for McpReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Report(_) => formatter.write_str("Report"),
            Self::Loaded(_) => formatter.write_str("Loaded"),
            Self::Changed(_) => formatter.write_str("Changed"),
            Self::LoginUrl { server_id, .. } => {
                write!(formatter, "LoginUrl({})", server_id.as_str())
            }
            Self::LoggedIn { server_id, .. } => {
                write!(formatter, "LoggedIn({})", server_id.as_str())
            }
            Self::Failed { error, .. } => write!(formatter, "Failed({error})"),
        }
    }
}

impl AgentBridge {
    pub(super) fn mcp_call(&mut self, call: McpCall) {
        match call {
            McpCall::Login(server_id) => self.mcp_login(server_id),
            // Dropping the sender ends the login's wait.
            McpCall::CancelLogin => self.mcp_login = None,
            call => self.spawn(move |agent| async move {
                let reply = run(&agent, call).await;
                Ok(Some(update(reply)))
            }),
        }
    }

    /// Logs in on the Agent's runtime: the address to open comes back as
    /// soon as there is one, and the login waits for the browser until it
    /// is cancelled (or another starts).
    fn mcp_login(&mut self, server_id: McpServerId) {
        let (cancel, cancelled) = oneshot::channel::<()>();
        self.mcp_login = Some(cancel);
        let app_event_tx = self.app_event_tx.clone();
        self.spawn(move |agent| async move {
            let opened = server_id.clone();
            let result = agent
                .mcp_oauth_login(
                    server_id.clone(),
                    move |url| {
                        app_event_tx.send(AppEvent::AgentReply(update(McpReply::LoginUrl {
                            server_id: opened,
                            url: url.to_string(),
                        })))
                    },
                    async move {
                        let _ = cancelled.await;
                    },
                )
                .await;
            Ok(Some(update(match result {
                Ok(snapshot) => McpReply::LoggedIn {
                    server_id,
                    snapshot,
                },
                Err(error) => McpReply::Failed {
                    error,
                    reloaded: None,
                },
            })))
        });
    }
}

fn update(reply: McpReply) -> AgentReply {
    AgentReply::Update(Box::new(TurnUpdate::Mcp(reply)))
}

async fn run(agent: &AgentHandle, call: McpCall) -> McpReply {
    let failed = |error: String| McpReply::Failed {
        error,
        reloaded: None,
    };
    match call {
        McpCall::Report => agent
            .mcp_snapshot()
            .await
            .map_or_else(failed, McpReply::Report),
        McpCall::Load => agent
            .mcp_snapshot()
            .await
            .map_or_else(failed, McpReply::Loaded),
        McpCall::Write {
            expected_revision,
            server,
            secrets,
            dropped,
        } => {
            let written = agent
                .mcp_write_server(McpServerWriteRequest {
                    expected_revision,
                    server: *server,
                })
                .await;
            match written {
                Ok(snapshot) => with_secrets(agent, snapshot, secrets, dropped).await,
                Err(error) => refused(agent, expected_revision, error).await,
            }
        }
        McpCall::Delete {
            expected_revision,
            server_id,
            dropped,
        } => {
            let deleted = agent
                .mcp_delete_server(McpServerDeleteRequest {
                    expected_revision,
                    server_id,
                })
                .await;
            match deleted {
                Ok(snapshot) => with_secrets(agent, snapshot, Vec::new(), dropped).await,
                Err(error) => refused(agent, expected_revision, error).await,
            }
        }
        McpCall::Connect(server_id) => agent
            .mcp_connect(server_id)
            .await
            .map_or_else(failed, McpReply::Changed),
        McpCall::Refresh(server_id) => agent
            .mcp_refresh(server_id)
            .await
            .map_or_else(failed, McpReply::Changed),
        McpCall::Logout(server_id) => agent
            .mcp_oauth_logout(server_id)
            .await
            .map_or_else(failed, McpReply::Changed),
        McpCall::Login(_) | McpCall::CancelLogin => {
            failed("the login runs from the bridge".to_string())
        }
    }
}

/// Sets `secrets`, then removes each of `dropped` no server references.
async fn with_secrets(
    agent: &AgentHandle,
    mut snapshot: McpApplicationSnapshot,
    secrets: Vec<(McpCredentialKey, String)>,
    dropped: Vec<McpCredentialKey>,
) -> McpReply {
    for (key, value) in secrets {
        match agent.mcp_set_credential(key.clone(), value).await {
            Ok(next) => snapshot = next,
            Err(error) => {
                return McpReply::Failed {
                    error: format!(
                        "the server was saved, but its secret {} was not: {error}",
                        key.as_str()
                    ),
                    reloaded: Some(snapshot),
                }
            }
        }
    }
    for key in dropped {
        let referenced = snapshot
            .settings
            .servers
            .iter()
            .any(|server| references(server, &key));
        if referenced || !snapshot.settings.credentials.contains_key(&key) {
            continue;
        }
        // A secret left behind is no harm to the server just saved.
        if let Ok(next) = agent.mcp_remove_credential(key).await {
            snapshot = next;
        }
    }
    McpReply::Changed(snapshot)
}

fn references(server: &McpServerConfig, key: &McpCredentialKey) -> bool {
    match &server.transport {
        McpTransportConfig::Stdio(stdio) => stdio.credential_environment.values().any(|k| k == key),
        McpTransportConfig::StreamableHttp(http) => {
            http.credential_headers.values().any(|k| k == key)
        }
    }
}

/// A refused write; with the settings when their revision moved.
async fn refused(agent: &AgentHandle, expected_revision: u64, error: String) -> McpReply {
    McpReply::Failed {
        error,
        reloaded: agent
            .mcp_snapshot()
            .await
            .ok()
            .filter(|snapshot| snapshot.settings.revision != expected_revision),
    }
}
