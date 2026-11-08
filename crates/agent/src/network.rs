//! The network settings the host owns and the Agent's HTTP clients follow.

/// The proxy the Agent's web tools, MCP servers and OAuth reach the network
/// through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentProxy {
    /// The operating system's proxy settings.
    System,
    /// No proxy.
    Disabled,
    /// This proxy URL.
    Explicit(String),
}

/// Where the host's network settings come from. Read when the Agent starts
/// and again whenever the host says its settings changed
/// (`AgentSlot::apply_saved_settings`); it must not block.
pub trait NetworkSettingsPort: Send + Sync {
    /// The proxy as the host's settings say now, or why they cannot say.
    fn proxy(&self) -> Result<AgentProxy, String>;
}
