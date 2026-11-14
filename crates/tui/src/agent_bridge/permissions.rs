// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/permissions` and `/approve` onto the facade: the session's permission
//! mode (`set_session_permission_mode`, the configured one when it has
//! none) and sandbox mode (`set_session_sandbox_mode`), and the approval of
//! an action automatic review denied
//! (`approve_denied_action`). The denials are the session's
//! `PermissionReviewCompleted { Denied }` events, as they arrive: the
//! latest ten of each session (Codex's `RecentAutoReviewDenials`).

use std::collections::{HashMap, VecDeque};

use hanafubuki_agent::sandbox::{platform_default_mode, SandboxStatus};
use hanafubuki_agent::{
    AgentEventKind, AgentPermissionMode, AgentSandboxMode, AgentSessionId, PermissionReviewStatus,
};

use super::{AgentBridge, SessionReply, TurnUpdate};

/// How many of a session's latest denials `/approve` offers, as many as
/// the facade accepts.
const RECENT_DENIALS: usize = 10;

/// An action automatic review denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeniedAction {
    pub(crate) review_id: String,
    pub(crate) summary: String,
    /// Why the review denied it, when it said.
    pub(crate) rationale: Option<String>,
}

/// The latest denials of each session, newest first.
#[derive(Debug, Default)]
pub(super) struct RecentDenials(HashMap<AgentSessionId, VecDeque<DeniedAction>>);

impl RecentDenials {
    /// Keeps a denied review of `session_id`'s; other events change nothing.
    pub(super) fn observe(&mut self, session_id: AgentSessionId, kind: &AgentEventKind) {
        let AgentEventKind::PermissionReviewCompleted {
            review_id,
            action_summary,
            status: PermissionReviewStatus::Denied,
            assessment,
            ..
        } = kind
        else {
            return;
        };
        let denials = self.0.entry(session_id).or_default();
        denials.retain(|denial| denial.review_id != *review_id);
        denials.push_front(DeniedAction {
            review_id: review_id.clone(),
            summary: action_summary.clone(),
            rationale: assessment
                .as_ref()
                .map(|assessment| assessment.rationale.clone()),
        });
        denials.truncate(RECENT_DENIALS);
    }

    pub(super) fn of(&self, session_id: AgentSessionId) -> Vec<DeniedAction> {
        self.0
            .get(&session_id)
            .map(|denials| denials.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn remove(&mut self, session_id: AgentSessionId, review_id: &str) {
        if let Some(denials) = self.0.get_mut(&session_id) {
            denials.retain(|denial| denial.review_id != review_id);
        }
    }
}

impl AgentBridge {
    /// The session's own mode, the configured one, and the sandbox mode in
    /// effect.
    pub(super) fn permission_modes(&self) -> Result<(), String> {
        let session_id = self.current_id()?;
        self.spawn_session_call("permissions", move |agent| async move {
            let read = |error| format!("the permission mode was not read: {error}");
            let session = agent.session_permission_mode(session_id).map_err(read)?;
            let session_sandbox = agent.session_sandbox_mode(session_id).map_err(read)?;
            let settings = agent.settings_view().await?.settings;
            let sandbox_status = agent.sandbox_status().map_err(read)?;
            Ok(SessionReply::PermissionModes {
                session,
                configured: settings.permission_mode,
                sandbox: session_sandbox
                    .or(settings.sandbox_mode)
                    .unwrap_or_else(platform_default_mode),
                sandbox_available: matches!(sandbox_status, SandboxStatus::Available { .. }),
                sandbox_reason: match sandbox_status {
                    SandboxStatus::Unavailable { reason } => Some(reason),
                    _ => None,
                },
            })
        });
        Ok(())
    }

    pub(super) fn setup_windows_sandbox(&self) {
        self.spawn_session_call("setup-default-sandbox", move |agent| async move {
            tokio::task::spawn_blocking(hanafubuki_agent::sandbox::setup_windows_sandbox)
                .await.map_err(|error| format!("sandbox setup worker: {error}"))??;
            let changes = [("windows.sandbox", "elevated"), ("sandboxMode", "workspace-write")]
                .into_iter().map(|(key, value)| hanafubuki_agent::SettingChange {
                    key: key.into(), op: hanafubuki_agent::SettingWriteOp::Set(value.into()),
                }).collect();
            let outcome = agent.write_setting_values(hanafubuki_agent::SettingsWrite { changes }).await?;
            if !outcome.shadowed.is_empty() {
                return Err("Sandbox installed, but a higher configuration layer overrides its settings; inspect /config windows".into());
            }
            Ok(SessionReply::WindowsSandboxReady)
        });
    }

    /// A preset: both of the session's modes. The next turn takes them.
    pub(super) fn set_preset(
        &self,
        mode: AgentPermissionMode,
        sandbox: AgentSandboxMode,
    ) -> Result<TurnUpdate, String> {
        let session_id = self.current_id()?;
        self.agent
            .set_session_sandbox_mode(session_id, sandbox)
            .map_err(|error| format!("the sandbox mode was not changed: {error}"))?;
        self.agent
            .set_session_permission_mode(session_id, mode)
            .map_err(|error| format!("the permission mode was not changed: {error}"))?;
        self.send_session_status();
        Ok(TurnUpdate::Session(SessionReply::PresetSet(mode, sandbox)))
    }

    pub(super) fn set_permission_mode(
        &self,
        mode: AgentPermissionMode,
    ) -> Result<TurnUpdate, String> {
        let session_id = self.current_id()?;
        self.agent
            .set_session_permission_mode(session_id, mode)
            .map_err(|error| format!("the permission mode was not changed: {error}"))?;
        // The status line's `permission-mode` item shows the new mode.
        self.send_session_status();
        Ok(TurnUpdate::Session(SessionReply::PermissionModeSet(mode)))
    }

    pub(super) fn denials(&self) -> TurnUpdate {
        let denials = self
            .session_id
            .map(|session_id| self.denials.of(session_id))
            .unwrap_or_default();
        TurnUpdate::Session(SessionReply::Denials(denials))
    }

    /// Approves the denial `review_id`; the facade refuses it while a turn
    /// runs, and says why.
    pub(super) fn approve_denial(&mut self, review_id: String) -> Result<TurnUpdate, String> {
        let session_id = self.current_id()?;
        let summary = self
            .denials
            .of(session_id)
            .into_iter()
            .find(|denial| denial.review_id == review_id)
            .map(|denial| denial.summary)
            .ok_or_else(|| "that denial is no longer available".to_string())?;
        self.agent
            .approve_denied_action(session_id, &review_id)
            .map_err(|error| format!("the action was not approved: {error}"))?;
        self.denials.remove(session_id, &review_id);
        Ok(TurnUpdate::Session(SessionReply::Approved(summary)))
    }
}

#[cfg(test)]
mod tests;
