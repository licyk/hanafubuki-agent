// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The settings center onto the facade's settings catalog: a read, a write
//! of settings by key, or an action on a collection, each a task of the
//! Agent's runtime that reads the catalog again after it and answers with
//! both. The answer is a workspace event, so it reaches the center
//! whichever conversation has the focus.

use hanafubuki_agent::{
    AgentHandle, AgentTuiPreferences, SettingsActionOutcome, SettingsActionRequest,
    SettingsCatalog, SettingsWrite, SettingsWriteOutcome,
};

use super::AgentBridge;
use crate::app::WorkspaceEvent;
use crate::app_event::AppEvent;

/// What the center asks.
#[derive(Debug)]
pub(crate) enum CatalogCall {
    Read,
    Write(SettingsWrite),
    Action(SettingsActionRequest),
}

/// What a call did, and the catalog read after it.
#[derive(Debug)]
pub(crate) struct CatalogReply {
    /// The number the center gave the call.
    pub(crate) request: u64,
    pub(crate) outcome: Result<CatalogOutcome, String>,
    pub(crate) catalog: Result<SettingsCatalog, String>,
    /// `[tui]` as the layers resolve it after a write of one of its keys,
    /// for the interface to take.
    pub(crate) preferences: Option<AgentTuiPreferences>,
}

#[derive(Debug)]
pub(crate) enum CatalogOutcome {
    Read,
    Written(SettingsWriteOutcome),
    Acted(SettingsActionOutcome),
}

impl AgentBridge {
    /// Runs `call` off the loop; its reply comes back as
    /// [`WorkspaceEvent::Settings`].
    pub(crate) fn catalog_call(&self, request: u64, call: CatalogCall) {
        let app_event_tx = self.app_event_tx.untagged();
        let agent = self.agent.clone();
        self.agent.runtime().handle().spawn(async move {
            let reply = run(&agent, request, call).await;
            app_event_tx.send(AppEvent::Workspace(WorkspaceEvent::Settings(Box::new(
                reply,
            ))));
        });
    }
}

async fn run(agent: &AgentHandle, request: u64, call: CatalogCall) -> CatalogReply {
    let mut tui_written = false;
    let outcome = match call {
        CatalogCall::Read => Ok(CatalogOutcome::Read),
        CatalogCall::Write(write) => {
            tui_written = write
                .changes
                .iter()
                .any(|change| change.key.starts_with("tui."));
            agent
                .write_setting_values(write)
                .await
                .map(CatalogOutcome::Written)
        }
        CatalogCall::Action(action) => agent
            .run_settings_action(action)
            .await
            .map(CatalogOutcome::Acted),
    };
    let preferences = match tui_written && outcome.is_ok() {
        true => agent.tui_preferences().await.ok(),
        false => None,
    };
    CatalogReply {
        request,
        outcome,
        catalog: agent.settings_catalog().await,
        preferences,
    }
}
