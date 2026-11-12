//! `/mcp` onto the facade's MCP calls: the report (`/mcp verbose`) and an
//! OAuth login, which waits for the browser and so is no action of the
//! settings catalog (the servers' writes are).

use std::fmt;

use hanafubuki_agent::{McpApplicationSnapshot, McpServerId};
use tokio::sync::oneshot;

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app_event::AppEvent;

/// What `/mcp` asks of the Agent.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum McpCall {
    /// The servers and their tools, written into the transcript.
    Report,
    Login(McpServerId),
    /// Ends the login that waits for the browser.
    CancelLogin,
}

/// What the Agent answered.
pub(crate) enum McpReply {
    Report(Box<McpApplicationSnapshot>),
    /// The login waits for the browser at this address.
    LoginUrl {
        server_id: McpServerId,
        url: String,
    },
    LoggedIn(McpServerId),
    Failed(String),
}

impl fmt::Debug for McpCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Report => formatter.write_str("Report"),
            Self::Login(id) => write!(formatter, "Login({})", id.as_str()),
            Self::CancelLogin => formatter.write_str("CancelLogin"),
        }
    }
}

/// Without the snapshot.
impl fmt::Debug for McpReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Report(_) => formatter.write_str("Report"),
            Self::LoginUrl { server_id, .. } => {
                write!(formatter, "LoginUrl({})", server_id.as_str())
            }
            Self::LoggedIn(server_id) => write!(formatter, "LoggedIn({})", server_id.as_str()),
            Self::Failed(error) => write!(formatter, "Failed({error})"),
        }
    }
}

impl AgentBridge {
    pub(super) fn mcp_call(&mut self, call: McpCall) {
        match call {
            McpCall::Report => self.spawn(move |agent| async move {
                Ok(Some(update(
                    agent
                        .mcp_snapshot()
                        .await
                        .map_or_else(McpReply::Failed, |snapshot| {
                            McpReply::Report(Box::new(snapshot))
                        }),
                )))
            }),
            McpCall::Login(server_id) => self.mcp_login(server_id),
            // Dropping the sender ends the login's wait.
            McpCall::CancelLogin => self.mcp_login = None,
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
                Ok(_) => McpReply::LoggedIn(server_id),
                Err(error) => McpReply::Failed(error),
            })))
        });
    }
}

fn update(reply: McpReply) -> AgentReply {
    AgentReply::Update(Box::new(TurnUpdate::Mcp(reply)))
}
