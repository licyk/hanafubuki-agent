//! The network settings the host owns and the Agent's HTTP clients follow.

/// The proxy the Agent's web tools, MCP servers and OAuth reach the network
/// through; the standalone Agent's `[network]` section holds it.
pub use hanafubuki_agent_config::AgentProxy;

/// Where the host's network settings come from. Read when the Agent starts
/// and again whenever the host says its settings changed
/// (`AgentSlot::apply_saved_settings`); it must not block.
pub trait NetworkSettingsPort: Send + Sync {
    /// The proxy as the host's settings say now, or why they cannot say.
    fn proxy(&self) -> Result<AgentProxy, String>;
}

/// The proxy the model API's clients take for `proxy`.
pub(crate) fn model_http_proxy(proxy: AgentProxy) -> hanafubuki_agent_kernel::ModelHttpProxy {
    use hanafubuki_agent_kernel::ModelHttpProxy;
    match proxy {
        AgentProxy::System => ModelHttpProxy::System,
        AgentProxy::Disabled => ModelHttpProxy::Disabled,
        AgentProxy::Explicit(address) => ModelHttpProxy::Explicit(address),
    }
}
