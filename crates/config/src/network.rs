//! The proxy the Agent's HTTP clients use. A host supplies it
//! (`NetworkSettingsPort`); the standalone Agent reads it from its
//! configuration's `[network]` section.

use hanafubuki_agent_foundation::url_policy::{UrlPolicy, MAX_URL_BYTES};
use serde::{Deserialize, Serialize};

/// The proxy the Agent's web tools, MCP servers and OAuth reach the network
/// through. In the configuration it is one string: `"system"`, `"disabled"`
/// or the proxy's URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum AgentProxy {
    /// The operating system's proxy settings.
    System,
    /// No proxy.
    Disabled,
    /// This proxy URL.
    Explicit(String),
}

/// A proxy URL: one of the schemes the HTTP clients support, with a host and
/// optionally credentials, and nothing after the address.
const PROXY_URL: UrlPolicy = UrlPolicy {
    schemes: &["http", "https", "socks4", "socks4a", "socks5", "socks5h"],
    credentials: true,
    query: false,
    fragment: false,
    max_bytes: Some(MAX_URL_BYTES),
};

impl TryFrom<String> for AgentProxy {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.trim() {
            "system" => Ok(Self::System),
            "disabled" => Ok(Self::Disabled),
            url => {
                PROXY_URL
                    .parse(url)
                    .map_err(|rejection| format!("the proxy {rejection}"))?;
                Ok(Self::Explicit(url.to_string()))
            }
        }
    }
}

impl From<AgentProxy> for String {
    fn from(proxy: AgentProxy) -> Self {
        match proxy {
            AgentProxy::System => "system".to_string(),
            AgentProxy::Disabled => "disabled".to_string(),
            AgentProxy::Explicit(url) => url,
        }
    }
}
