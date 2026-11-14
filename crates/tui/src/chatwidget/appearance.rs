// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The chat's status line and terminal title (Codex's
//! `chatwidget/status_surfaces.rs`): they show the session's status as the
//! bridge reads it, with the lists `/statusline` and `/title` set.

use std::time::Instant;

use hanafubuki_agent::AgentTuiPreferences;

use super::ChatWidget;
use crate::agent_bridge::SessionStatus;
use crate::appearance::AppearanceEvent;

impl ChatWidget {
    /// A change of the theme, the status line or the title.
    pub(crate) fn apply_appearance(&mut self, event: AppearanceEvent) {
        self.appearance.apply(event);
        self.sync_status_line();
    }

    /// The theme, the status line and the title `preferences` hold.
    pub(crate) fn adopt_appearance(&mut self, preferences: &AgentTuiPreferences) {
        self.appearance.adopt(preferences);
        self.sync_status_line();
    }

    pub(super) fn set_session_status(&mut self, status: SessionStatus) {
        self.appearance.values_mut().status = Some(status);
        self.sync_status_line();
    }

    /// The status line as the settings and the session have it now, and
    /// the update notice beside it.
    pub(super) fn sync_status_line(&mut self) {
        self.bottom_pane
            .set_status_line(self.appearance.status_line());
        self.bottom_pane
            .set_update_notice(self.appearance.update_notice().cloned());
    }

    /// The terminal's title at `now`, and whether it changes with time.
    pub(crate) fn terminal_title(&mut self, now: Instant) -> (String, bool) {
        self.appearance.values_mut().task_running = self.bottom_pane.is_task_running();
        (
            self.appearance.terminal_title(now),
            self.appearance.title_is_animated(),
        )
    }
}
