//! What the plugin's registrations share: the hooks as the latest
//! discovery found them, and the runs of an event on the blocking pool.

use std::collections::HashMap;
use std::sync::Arc;

use hanafubuki_agent_contracts::{AgentSessionId, AgentTurnId, BlockingService, CancellationToken};
use hanafubuki_agent_hooks::{
    HookCatalog, HookEvent, HookEventName, HookOutcome, HookScope, HooksEngine,
};
use parking_lot::{Mutex, RwLock};

use crate::plugin::WorkingDirectory;

pub(crate) struct HooksRuntime {
    pub(crate) engine: Arc<HooksEngine>,
    working_directory: WorkingDirectory,
    blocking: Arc<BlockingService>,
    /// Discovered again at each turn's admission and after each change of
    /// trust or enablement, so a turn's calls see one configuration.
    catalog: RwLock<Arc<HookCatalog>>,
    /// The context each session's SessionStart hooks gave, once per session
    /// while the plugin runs.
    pub(crate) started: Mutex<HashMap<AgentSessionId, Option<String>>>,
    /// The turn of each session a Stop hook last kept going.
    pub(crate) continued: Mutex<HashMap<AgentSessionId, AgentTurnId>>,
}

impl HooksRuntime {
    pub(crate) fn new(
        engine: Arc<HooksEngine>,
        working_directory: WorkingDirectory,
        blocking: Arc<BlockingService>,
    ) -> Self {
        Self {
            engine,
            working_directory,
            blocking,
            catalog: RwLock::new(Arc::new(HookCatalog::default())),
            started: Mutex::new(HashMap::new()),
            continued: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn catalog(&self) -> Arc<HookCatalog> {
        self.catalog.read().clone()
    }

    /// Whether any hook of `event` runs for `inputs`.
    pub(crate) fn has_runnable(&self, event: HookEventName, inputs: &[&str]) -> bool {
        self.catalog().runnable(event, inputs).next().is_some()
    }

    /// Discovers the hooks again, on the blocking pool.
    pub(crate) async fn refresh(&self) -> Result<Arc<HookCatalog>, String> {
        let engine = self.engine.clone();
        let catalog = self
            .blocking
            .run("agent hooks discovery", move || Arc::new(engine.discover()))
            .await
            .map_err(|error| error.to_string())?;
        for diagnostic in &catalog.diagnostics {
            log::warn!("hooks: {diagnostic}");
        }
        *self.catalog.write() = catalog.clone();
        Ok(catalog)
    }

    /// Runs `event`'s hooks of the current catalog, on the blocking pool,
    /// in the working directory.
    pub(crate) async fn run(
        &self,
        event: HookEvent,
        session_id: AgentSessionId,
        turn_id: Option<AgentTurnId>,
        cancellation: CancellationToken,
    ) -> Result<HookOutcome, String> {
        let engine = self.engine.clone();
        let catalog = self.catalog();
        let working_directory = self.working_directory.clone();
        self.blocking
            .run("agent hooks", move || {
                let scope = HookScope {
                    session_id: session_id.to_string(),
                    turn_id: turn_id.map(|turn| turn.to_string()),
                    cwd: working_directory()?,
                };
                Ok(engine.run(&catalog, &event, &scope, &cancellation))
            })
            .await
            .map_err(|error| error.to_string())?
    }
}

/// `text` cut to at most `limit` bytes on a character boundary.
pub(crate) fn bounded(mut text: String, limit: usize) -> String {
    if text.len() > limit {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}
