// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The chat's status line and terminal title (Codex's
//! `chatwidget/status_surfaces.rs`): they show the session's status as the
//! bridge reads it, with the lists `/statusline` and `/title` set.

use std::time::Instant;

use super::ChatWidget;
use crate::agent_bridge::SessionStatus;
use crate::appearance::AppearanceEvent;
use crate::history_cell;

impl ChatWidget {
    /// A change of the theme, the status line or the title.
    pub(crate) fn apply_appearance(&mut self, event: AppearanceEvent) {
        self.appearance.apply(event);
        self.sync_status_line();
    }

    pub(super) fn set_session_status(&mut self, status: SessionStatus) {
        self.appearance.values_mut().status = Some(status);
        self.sync_status_line();
    }

    /// The status line as the settings and the session have it now.
    pub(super) fn sync_status_line(&mut self) {
        self.bottom_pane
            .set_status_line(self.appearance.status_line());
    }

    /// The terminal's title at `now`, and whether it changes with time.
    pub(crate) fn terminal_title(&mut self, now: Instant) -> (String, bool) {
        self.appearance.values_mut().task_running = self.bottom_pane.is_task_running();
        (
            self.appearance.terminal_title(now),
            self.appearance.title_is_animated(),
        )
    }

    /// Something the interface could not apply as configured: shown, and
    /// kept for `/warnings`.
    pub(crate) fn add_warning(&mut self, warning: String) {
        self.add_to_history(history_cell::new_error_event(warning.clone()));
        self.warnings.push(warning);
        self.request_redraw();
    }
}
