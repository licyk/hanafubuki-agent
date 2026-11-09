use std::sync::Arc;

use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_hooks::{HookCatalog, HooksEngine};

use crate::runtime::HooksRuntime;

/// What the `hooks` row provides: the hooks configured now, and changes to
/// their enablement and trust, on the blocking pool. A change takes effect
/// at once: the running turns' calls see it too. A failure is the
/// operation's message.
pub struct HooksService {
    pub(crate) runtime: Arc<HooksRuntime>,
    pub(crate) blocking: Arc<BlockingService>,
}

impl HooksService {
    /// Every configured hook, by source in configuration order, with its
    /// trust and enablement, and what could not be used.
    pub async fn list(&self) -> Result<Arc<HookCatalog>, String> {
        self.runtime.refresh().await
    }

    /// Turns the hook `key` on or off.
    pub async fn set_enabled(&self, key: String, enabled: bool) -> Result<(), String> {
        self.change("agent hooks enable", move |engine| {
            engine.set_enabled(&key, enabled)
        })
        .await
    }

    /// Trusts each hook of `keys` as it is defined now.
    pub async fn trust(&self, keys: Vec<String>) -> Result<(), String> {
        self.change("agent hooks trust", move |engine| engine.trust(&keys))
            .await
    }

    async fn change(
        &self,
        label: &'static str,
        change: impl FnOnce(&HooksEngine) -> Result<(), String> + Send + 'static,
    ) -> Result<(), String> {
        let engine = self.runtime.engine.clone();
        self.blocking
            .run(label, move || change(&engine))
            .await
            .map_err(|error| error.to_string())??;
        self.runtime.refresh().await.map(|_| ())
    }
}
