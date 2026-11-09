// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Editing an earlier message (Codex's `app_backtrack`).
//!
//! - A first Esc on an empty composer, with nothing running and nothing
//!   open, primes: the footer says a second Esc edits a previous message
//!   (and a scrolled owned transcript returns to its latest rows). Any
//!   other key cancels.
//! - The second Esc browses the prompts, the latest highlighted: in place
//!   in the owned transcript, or in the transcript overlay in terminal
//!   mode, in the compact presentation. ←/→ (h/l) choose the prompt, ↑/↓
//!   (j/k) scroll, Ctrl+T toggles the details, and Esc returns to where
//!   browsing started. In the owned transcript the view's own keys (paging,
//!   the jumps, find) still work and any other key ends browsing; in the
//!   overlay the overlay has them.
//! - Enter edits the chosen prompt: the bridge finds its turn and the
//!   facade forks the session before it and archives the original
//!   (`edit_from_turn`); the interface switches to the fork, whose
//!   transcript ends before that prompt, and the prompt is back in the
//!   composer. Files are not restored, which the first edit of the run
//!   says.
//!
//! The chosen prompt is kept as a count from the latest, never as a cell
//! index, so cells that arrive (or earlier pages spliced in) while browsing
//! do not move it. Codex's browsing from an open Ctrl+T transcript,
//! steering-aware prompt restoration, mention bindings and side
//! conversations are not here.

use std::io::{self, Write};
use std::time::Instant;

use ratatui::backend::Backend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Stylize;
use ratatui::text::Line;
use unicode_width::UnicodeWidthStr;

use super::{is_transcript_key, App};
use crate::agent_bridge::AgentReply;
use crate::history_cell::UserHistoryCell;
use crate::pager_overlay::{Overlay, TranscriptOverlay};
use crate::transcript_view::TranscriptBookmark;
use crate::tui::Tui;

const NO_PREVIOUS_MESSAGE: &str = "No previous message to edit.";

#[derive(Default)]
pub(super) struct Backtrack {
    /// A first Esc came; a second one browses.
    primed: bool,
    /// While browsing: the chosen prompt, counted from the latest (0).
    browsing: Option<usize>,
    /// Where the owned transcript was read when the first Esc came.
    origin: Option<TranscriptBookmark>,
    /// The first edit of the run said that files are not restored.
    files_noted: bool,
}

impl Backtrack {
    pub(super) fn is_browsing(&self) -> bool {
        self.browsing.is_some()
    }

    #[cfg(test)]
    pub(super) fn is_primed(&self) -> bool {
        self.primed
    }
}

impl App {
    /// `true` when `key` primed, browsed or edited.
    pub(super) fn handle_backtrack_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if self.backtrack.browsing.is_some() {
            return self.handle_browsing_key(tui, key);
        }
        if key.code == KeyCode::Esc && key.modifiers == KeyModifiers::NONE && self.can_prime() {
            if key.kind == KeyEventKind::Press {
                if self.backtrack.primed {
                    self.begin_browsing(tui);
                } else {
                    self.prime();
                }
                self.schedule_frame(Instant::now());
            }
            return true;
        }
        if self.backtrack.primed {
            self.reset_backtrack();
        }
        false
    }

    /// A paste cancels a first Esc, as a key does.
    pub(super) fn cancel_primed_backtrack(&mut self) {
        if self.backtrack.primed && self.backtrack.browsing.is_none() {
            self.reset_backtrack();
        }
    }

    pub(super) fn reset_backtrack(&mut self) {
        self.backtrack = Backtrack {
            files_noted: self.backtrack.files_noted,
            ..Backtrack::default()
        };
        self.transcript_view.set_highlight(None);
        self.chat_widget.clear_esc_backtrack_hint();
    }

    /// Highlights the chosen prompt again in the owned transcript, whose
    /// cells may have moved (an earlier page was spliced in).
    pub(super) fn sync_browsing_highlight(&mut self) {
        if self.overlay.is_some() {
            return;
        }
        let index = self
            .backtrack
            .browsing
            .and_then(|nth| self.prompt_indices().get(nth).copied());
        self.transcript_view.set_highlight(index);
    }

    fn can_prime(&self) -> bool {
        self.overlay.is_none()
            && self.resume_picker.is_none()
            && self.chat_widget.no_modal_or_popup_active()
            && self.chat_widget.composer_is_empty()
            && !self.chat_widget.is_task_running()
            && !(self.mode.is_owned()
                && (self.transcript_view.has_active_interaction()
                    || self.transcript_view.is_detailed()))
    }

    fn prime(&mut self) {
        self.backtrack.primed = true;
        if self.mode.is_owned() {
            self.backtrack.origin = Some(self.transcript_view.bookmark());
            self.transcript_view.jump_to_latest();
        }
        if !self.prompt_indices().is_empty() {
            self.chat_widget.show_esc_backtrack_hint();
        }
    }

    fn begin_browsing<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        if self.prompt_indices().is_empty() {
            self.reset_backtrack();
            self.chat_widget
                .add_info_message(NO_PREVIOUS_MESSAGE.to_string());
            return;
        }
        self.chat_widget.clear_esc_backtrack_hint();
        self.backtrack.browsing = Some(0);
        if !self.mode.is_owned() {
            let mut overlay =
                TranscriptOverlay::new(self.transcript_cells.clone(), self.render_mode);
            overlay.set_presentation(false, self.render_mode);
            self.open_overlay(tui, Overlay::Transcript(overlay));
        }
        self.show_browsing_choice(false);
    }

    fn handle_browsing_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        let interacting = match &self.overlay {
            Some(Overlay::Transcript(overlay)) => overlay.has_active_interaction(),
            _ => self.transcript_view.has_active_interaction(),
        };
        // A selection or a search keeps its keys.
        if interacting {
            return false;
        }
        let press = key.kind == KeyEventKind::Press;
        let plain = key.modifiers == KeyModifiers::NONE;
        match key.code {
            _ if is_transcript_key(key) => self.toggle_browsing_details(),
            KeyCode::Enter if plain => {
                if press {
                    self.confirm_browsing(tui);
                }
            }
            KeyCode::Esc if plain => {
                if press {
                    self.cancel_browsing(tui);
                }
            }
            KeyCode::Left | KeyCode::Char('h') if plain => self.step_browsing(true),
            KeyCode::Right | KeyCode::Char('l') if plain => self.step_browsing(false),
            KeyCode::Up | KeyCode::Char('k') if plain => self.scroll_browsing(-1),
            KeyCode::Down | KeyCode::Char('j') if plain => self.scroll_browsing(1),
            // The overlay pages, jumps, finds and closes.
            _ if self.overlay.is_some() => return false,
            _ => match self.transcript_view.handle_key(key, &self.transcript_cells) {
                Some(action) => self.apply_view_action(action),
                None => {
                    self.cancel_browsing(tui);
                    return false;
                }
            },
        }
        self.schedule_frame(Instant::now());
        true
    }

    /// The indices of the prompt cells, latest first.
    fn prompt_indices(&self) -> Vec<usize> {
        self.transcript_cells
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, cell)| cell.as_any().is::<UserHistoryCell>())
            .map(|(index, _)| index)
            .collect()
    }

    /// Highlights the chosen prompt and brings it into view; `jump` shows
    /// it from its top even when some of it is in view.
    fn show_browsing_choice(&mut self, jump: bool) {
        let Some(nth) = self.backtrack.browsing else {
            return;
        };
        let Some(&index) = self.prompt_indices().get(nth) else {
            return;
        };
        match self.overlay.as_mut() {
            Some(Overlay::Transcript(overlay)) => overlay.highlight(index, jump),
            _ => {
                let view = &mut self.transcript_view;
                view.set_highlight(Some(index));
                if jump {
                    view.jump_to_entry(&self.transcript_cells, index);
                } else {
                    view.ensure_entry_visible(&self.transcript_cells, index);
                }
            }
        }
    }

    /// One prompt earlier, or later.
    fn step_browsing(&mut self, earlier: bool) {
        let Some(nth) = self.backtrack.browsing else {
            return;
        };
        let last = self.prompt_indices().len().saturating_sub(1);
        let next = if earlier {
            (nth + 1).min(last)
        } else {
            nth.saturating_sub(1)
        };
        self.backtrack.browsing = Some(next);
        self.show_browsing_choice(false);
    }

    fn scroll_browsing(&mut self, rows: isize) {
        match self.overlay.as_mut() {
            Some(Overlay::Transcript(overlay)) => overlay.scroll(rows),
            _ => self.transcript_view.scroll(&self.transcript_cells, rows),
        }
    }

    fn toggle_browsing_details(&mut self) {
        let mode = self.render_mode;
        match self.overlay.as_mut() {
            Some(Overlay::Transcript(overlay)) => {
                let detailed = !overlay.is_detailed();
                overlay.set_presentation(detailed, mode);
            }
            _ => {
                let detailed = !self.transcript_view.is_detailed();
                self.transcript_view.set_presentation(detailed, mode);
            }
        }
        self.show_browsing_choice(true);
    }

    /// Back to where browsing started.
    fn cancel_browsing<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        let origin = self.backtrack.origin.take();
        self.reset_backtrack();
        if self.overlay.take().is_some() {
            self.leave_alt_screen(tui);
        } else if let Some(origin) = origin {
            self.transcript_view
                .set_presentation(false, self.render_mode);
            self.transcript_view.restore_bookmark(origin);
        }
    }

    /// Edits the chosen prompt in a fork of the session.
    fn confirm_browsing<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        let Some(nth) = self.backtrack.browsing else {
            return;
        };
        let prompts: Vec<String> = self
            .prompt_indices()
            .into_iter()
            .take(nth + 1)
            .filter_map(|index| {
                self.transcript_cells[index]
                    .as_any()
                    .downcast_ref::<UserHistoryCell>()
                    .map(|cell| cell.message.clone())
            })
            .collect();
        self.cancel_browsing(tui);
        if self.chat_widget.is_task_running() {
            self.chat_widget.add_error_message(
                "A turn is running: wait for it to end, or interrupt it, before editing an \
                 earlier message."
                    .to_string(),
            );
            return;
        }
        let edited = match self.bridge.edit_prompt(&prompts) {
            Ok(edited) => edited,
            Err(error) => {
                self.chat_widget
                    .add_error_message(format!("The message was not edited: {error}"));
                return;
            }
        };
        if let Some(update) = self.bridge.reply(AgentReply::SessionOpened(edited.opened)) {
            self.show_update(update);
        }
        self.chat_widget.set_composer_text(edited.prompt);
        let mut note = "The conversation goes on from a copy made before this message; the \
                        original is archived."
            .to_string();
        if edited.images > 0 {
            note.push_str(&format!(
                " Its {} image(s) are not restored: the composer cannot attach images.",
                edited.images
            ));
        }
        if !self.backtrack.files_noted {
            self.backtrack.files_noted = true;
            note.push_str(" Files are not restored.");
        }
        self.chat_widget.add_info_message(note);
    }
}

/// The browsing hints, the longest that fits in `width`.
pub(super) fn browsing_footer(width: u16) -> Line<'static> {
    const LABEL: &str = "Browsing";
    const HINTS: [&str; 4] = [
        " · ←/→ prompt · ↑/↓ scroll · ctrl+t details · ↵ edit · esc back",
        " · ←/→ prompt · ↵ edit · esc back",
        " · ↵ edit · esc back",
        "",
    ];
    let hints = HINTS
        .into_iter()
        .find(|hints| LABEL.width() + hints.width() <= usize::from(width))
        .unwrap_or_default();
    Line::from(vec![LABEL.fg(crate::style::accent_color()), hints.dim()])
}
