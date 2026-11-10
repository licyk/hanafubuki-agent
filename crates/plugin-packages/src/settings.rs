//! Where the guest host's loader tree is persisted: the Agent's settings,
//! through a port the facade provides (only the facade writes the
//! configuration).

use std::sync::Arc;

use kardia_loader::{EntryChange, TreeBackend};
use serde_json::Value;

/// The settings the guest host's tree is kept in.
pub trait GuestSettingsPort: Send + Sync + 'static {
    /// Records whether the package `id` (`<plugin>@<marketplace>`) is on.
    /// It is called while kardia-loader applies a change, so it must not
    /// block: the facade queues the write for its blocking writer, which
    /// writes only a value that differs from the settings'.
    fn set_enabled(&self, id: &str, enabled: bool);
}

/// The tree's backend. The entries mirror the settings: the guest host
/// applies them with a group update, which commits nothing. What
/// kardia-loader changes on its own (a guest that disposed itself is
/// written back as disabled) reaches the settings here.
pub(crate) struct SettingsTree {
    port: Arc<dyn GuestSettingsPort>,
}

impl SettingsTree {
    pub(crate) fn new(port: Arc<dyn GuestSettingsPort>) -> Self {
        Self { port }
    }
}

impl TreeBackend for SettingsTree {
    fn commit(&self, change: &EntryChange) {
        let Some(options) = &change.options else {
            return;
        };
        let disabled = |options: &Value| {
            options
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        if change.legacy.as_ref().map(disabled) == Some(disabled(options)) {
            return;
        }
        self.port.set_enabled(&change.id, !disabled(options));
    }
}
