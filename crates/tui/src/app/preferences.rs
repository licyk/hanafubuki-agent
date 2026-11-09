// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What `/raw` and `/tui` change: the presentation of the transcript now
//! (raw), and the transcript's mode at the next launch (Codex's
//! `app/tui_mode_picker.rs`); each is saved in `[tui]`.

use hanafubuki_agent::AgentTuiPreferencesPatch;

use super::App;
use crate::history_cell::HistoryRenderMode;
use crate::pager_overlay::Overlay;

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
