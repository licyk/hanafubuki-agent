//! The Agent's extension points, where the kernel calls plugin code.
//!
//! The kernel calls plugin code at its extension points through the guards
//! of `hanafubuki-agent-host` ([`crate::PluginFaults::guard`],
//! [`crate::RegistryEntry::guard`] and their async forms): a panic there
//! becomes that call's error and a [`crate::PluginFault`] of the owning
//! plugin at its [`ExtensionPoint`], and the Agent host stops the plugin.
//! One `PluginFaults` is shared by the kernel's registries and published by
//! the kernel plugin.

use std::fmt;

use hanafubuki_agent_host::FaultPoint;

/// The kind of plugin code a fault happened in. A panic in a plugin's
/// scoped worker is reported at `FaultPoint::worker(label)` instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionPoint {
    ToolAvailability,
    ToolPermission,
    ToolInvoke,
    ToolSource,
    Context,
    TurnAdmission,
    PromptPrepare,
    PromptContribute,
    ApprovalRule,
    ContinuationHook,
    Observer,
}

impl From<ExtensionPoint> for FaultPoint {
    fn from(point: ExtensionPoint) -> Self {
        FaultPoint::new(match point {
            ExtensionPoint::ToolAvailability => "tool availability",
            ExtensionPoint::ToolPermission => "tool permission",
            ExtensionPoint::ToolInvoke => "tool invoke",
            ExtensionPoint::ToolSource => "tool source",
            ExtensionPoint::Context => "context contributor",
            ExtensionPoint::TurnAdmission => "turn admission contributor",
            ExtensionPoint::PromptPrepare => "prompt prepare",
            ExtensionPoint::PromptContribute => "prompt contribute",
            ExtensionPoint::ApprovalRule => "approval rule",
            ExtensionPoint::ContinuationHook => "continuation hook",
            ExtensionPoint::Observer => "observer",
        })
    }
}

impl fmt::Display for ExtensionPoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&FaultPoint::from(*self), formatter)
    }
}
