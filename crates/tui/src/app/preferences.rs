// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What `/raw` and `/tui` change: the presentation of the transcript now
//! (raw), and the transcript's mode at the next launch (Codex's
//! `app/tui_mode_picker.rs`); each is saved in `[tui]`. And what `/theme`,
//! `/statusline` and `/title` change, saved when their views say so, and
//! the terminal's title, written when it changes and given back at exit.

use std::io::{self, Write};
use std::time::Instant;

use hanafubuki_agent::AgentTuiPreferencesPatch;
use ratatui::backend::Backend;

use super::App;
use crate::appearance::items::ids;
use crate::appearance::{AppearanceEvent, SPINNER_INTERVAL};
use crate::history_cell::HistoryRenderMode;
use crate::pager_overlay::Overlay;
use crate::tui::Tui;

impl App {
    /// `/raw`: on, off, or (`None`) the other way.
    pub(super) fn set_raw_output(&mut self, raw: Option<bool>) {
        let raw = raw.unwrap_or(self.render_mode == HistoryRenderMode::Rich);
        self.render_mode = if raw {
            HistoryRenderMode::Raw
        } else {
            HistoryRenderMode::Rich
        };
        let view = &mut self.transcript_view;
        view.set_presentation(view.is_detailed(), self.render_mode);
        if let Some(Overlay::Transcript(transcript)) = self.overlay.as_mut() {
            transcript.set_render_mode(self.render_mode);
        }
        let state = if raw { "on" } else { "off" };
        let message = if self.mode.is_owned() {
            format!("Raw output {state}.")
        } else {
            format!("Raw output {state} for what follows; the scrollback keeps what it shows.")
        };
        self.save(
            AgentTuiPreferencesPatch {
                raw_output: Some(raw),
                ..AgentTuiPreferencesPatch::default()
            },
            message,
        );
    }

    /// `/tui`: the transcript's mode from the next launch on.
    pub(super) fn save_fullscreen_transcript(&mut self, enabled: bool) {
        let mode = if enabled { "Fullscreen" } else { "Scrollback" };
        self.chat_widget.set_fullscreen_preference(enabled);
        self.save(
            AgentTuiPreferencesPatch {
                fullscreen_transcript: Some(enabled),
                ..AgentTuiPreferencesPatch::default()
            },
            format!(
                "Saved the interface mode: {mode}. It applies at the next launch; \
                 --no-alt-screen and tui.alternateScreen still apply."
            ),
        );
    }

    /// `/theme`, `/statusline`, `/title`: applied, and saved when asked.
    pub(super) fn apply_appearance(&mut self, event: AppearanceEvent) {
        let save = match &event {
            AppearanceEvent::ThemeSaved(name) => Some((
                AgentTuiPreferencesPatch {
                    theme: Some(name.clone()),
                    ..AgentTuiPreferencesPatch::default()
                },
                format!("Theme set to {name}."),
            )),
            AppearanceEvent::StatusLine {
                items,
                colors,
                save: true,
            } => Some((
                AgentTuiPreferencesPatch {
                    status_line: items.as_deref().map(ids),
                    status_line_colors: *colors,
                    ..AgentTuiPreferencesPatch::default()
                },
                "Status line updated.".to_string(),
            )),
            AppearanceEvent::TerminalTitle { items, save: true } => Some((
                AgentTuiPreferencesPatch {
                    terminal_title: Some(ids(items)),
                    ..AgentTuiPreferencesPatch::default()
                },
                "Terminal title updated.".to_string(),
            )),
            AppearanceEvent::StatusLine { save: false, .. }
            | AppearanceEvent::TerminalTitle { save: false, .. }
            | AppearanceEvent::PreviewTerminalTitle(_) => None,
        };
        let session_only = matches!(
            event,
            AppearanceEvent::StatusLine { save: false, .. }
                | AppearanceEvent::TerminalTitle { save: false, .. }
        );
        self.chat_widget.apply_appearance(event);
        match save {
            Some((patch, message)) => self.save(patch, message),
            None if session_only => self
                .chat_widget
                .add_info_message("Updated for this session; it is not saved.".to_string()),
            None => {}
        }
        self.schedule_frame(Instant::now());
    }

    /// Writes the terminal's title when it changed; while it turns with a
    /// spinner, the next frame comes with its next step.
    pub(super) fn sync_terminal_title<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) {
        let now = Instant::now();
        let (title, animated) = self.chat_widget.terminal_title(now);
        let _ = self.terminal_title.sync(tui.terminal.backend_mut(), &title);
        if animated {
            self.schedule_frame(now + SPINNER_INTERVAL);
        }
    }

    /// The terminal's own title back, as the interface exits.
    pub(crate) fn restore_terminal_title<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) {
        let _ = self.terminal_title.restore(tui.terminal.backend_mut());
    }

    /// Writes `patch` into `config.toml`, then says `message`, or why it
    /// was not saved.
    fn save(&mut self, patch: AgentTuiPreferencesPatch, message: String) {
        match self.bridge.write_tui_preferences(patch) {
            Ok(()) => self.chat_widget.add_info_message(message),
            Err(error) => self
                .chat_widget
                .add_error_message(format!("{message} It was not saved: {error}")),
        }
    }
}
