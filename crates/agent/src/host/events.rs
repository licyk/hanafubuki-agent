//! The hint the host gives when a plugin's state may have changed.

/// Told when a plugin was enabled, disabled, retried or reconfigured, when
/// such a change settled, and when a plugin's warning (its issues) appeared,
/// changed or cleared. It carries nothing: the interface reads the plugin
/// list again. Implemented by a root adapter; it must not block, and it may
/// be called on any thread.
pub trait AgentPluginEventSink: Send + Sync {
    fn plugins_changed(&self);
}

/// For hosts nobody watches (tests).
pub struct NoopAgentPluginEventSink;

impl AgentPluginEventSink for NoopAgentPluginEventSink {
    fn plugins_changed(&self) {}
}
