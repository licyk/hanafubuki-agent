//! The hint the host gives when a plugin's state may have changed, and the
//! changes to the installed plugins.

use crate::packages::AgentPackageChange;

/// Told when a plugin was enabled, disabled, retried or reconfigured, when
/// such a change settled, and when a plugin's warning (its issues) appeared,
/// changed or cleared: `plugins_changed` carries nothing, the interface
/// reads the plugin list again. Told too of each change to an installed
/// plugin (`package_changed`): installed, removed, enabled, disabled,
/// updated, or a call on it that failed. Implemented by a root adapter; it
/// must not block, and it may be called on any thread.
pub trait AgentPluginEventSink: Send + Sync {
    fn plugins_changed(&self);

    /// An installed plugin changed, after the Agent reloaded the installed
    /// plugins; `packages()` shows it as it is now.
    fn package_changed(&self, change: &AgentPackageChange) {
        let _ = change;
    }
}

/// For hosts nobody watches (tests).
pub struct NoopAgentPluginEventSink;

impl AgentPluginEventSink for NoopAgentPluginEventSink {
    fn plugins_changed(&self) {}
}
