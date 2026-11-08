//! Kernel faults: a panic the kernel caught in its own code.
//!
//! A panic in plugin code is that plugin's fault (`PluginFaults`): the
//! registries' guards catch it before it reaches the kernel. A panic that
//! reaches one of the kernel's own catches (a turn's task) happened in kernel
//! code, which may have left in-memory session state half-changed: the
//! kernel's locks do not poison, so nothing rolls back. The kernel reports it
//! to the sink its owner injected; the application's answer is to rebuild the
//! Agent from disk.

/// Told of every panic the kernel caught in its own code. Called on the
/// thread that caught it (a turn's task), so it only hands the report on.
pub trait AgentKernelFaultSink: Send + Sync {
    /// `site` names the catch (`"turn"`); `message` is the bounded panic
    /// message.
    fn kernel_fault(&self, site: &str, message: &str);
}

/// Faults are only logged.
pub struct NoopAgentKernelFaultSink;

impl AgentKernelFaultSink for NoopAgentKernelFaultSink {
    fn kernel_fault(&self, _site: &str, _message: &str) {}
}
