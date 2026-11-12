// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What `/raw` and `/tui` change: the presentation of the transcript now
//! (raw), and the transcript's mode at the next launch (Codex's
//! `app/tui_mode_picker.rs`); each is saved in `[tui]`. And what `/theme`,
//! `/statusline` and `/title` change, saved when their views say so, and
//! the terminal's title, written when it changes and given back at exit.

use std::io::{self, Write};
use std::time::Instant;

use hanafubuki_agent::{AgentTuiPreferences, AgentTuiPreferencesPatch};
use ratatui::backend::Backend;

use super::App;
use crate::appearance::items::ids;
use crate::appearance::{AppearanceEvent, SPINNER_INTERVAL};
use crate::history_cell::HistoryRenderMode;
use crate::keymap::{self, Keymap};
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

    /// Writes `patch` into `config.toml` and applies what it changed, then
    /// says `message`, or why it was not saved.
    pub(super) fn save(&mut self, patch: AgentTuiPreferencesPatch, mut message: String) {
        match self.bridge.write_tui_preferences(patch.clone()) {
            Ok(preferences) => {
                if shadowed(&patch, &preferences) {
                    message.push_str(
                        " A profile, the project's configuration or -c sets it too, and that \
                         one applies.",
                    );
                }
                // What the keymap cannot use was kept for `/warnings` at
                // start, and `/keymap` refuses what it would add.
                self.apply_preferences(preferences);
                self.chat_widget.add_info_message(message);
            }
            Err(error) => self
                .chat_widget
                .add_error_message(format!("{message} It was not saved: {error}")),
        }
    }

    /// Makes `preferences` the ones in force: the keymap they resolve to,
    /// the presets and the rest the commands read. Returns what of the
    /// keymap could not be used.
    pub(crate) fn apply_preferences(&mut self, preferences: AgentTuiPreferences) -> Vec<String> {
        let (keymap, warnings) = Keymap::resolve(&preferences.keymap);
        keymap::install(keymap);
        self.chords.clear();
        self.tail_mouse.setting = preferences.scrollback_mouse;
        self.chat_widget.set_preferences(preferences);
        warnings
    }
}

/// Whether a keymap entry or a preset `patch` wrote resolves to something
/// else: a layer above the user's file sets it.
pub(super) fn shadowed(
    patch: &AgentTuiPreferencesPatch,
    preferences: &AgentTuiPreferences,
) -> bool {
    let scalars = patch
        .fullscreen_transcript
        .is_some_and(|value| preferences.fullscreen_transcript != value)
        || patch
            .alternate_screen
            .is_some_and(|value| preferences.alternate_screen != value)
        || patch
            .scrollback_mouse
            .is_some_and(|value| preferences.scrollback_mouse != value)
        || patch
            .raw_output
            .is_some_and(|value| preferences.raw_output != value)
        || patch
            .status_line_colors
            .is_some_and(|value| preferences.status_line_colors != value)
        || patch
            .theme
            .as_ref()
            .is_some_and(|value| preferences.theme.as_ref() != Some(value))
        || patch
            .status_line
            .as_ref()
            .is_some_and(|value| preferences.status_line != *value)
        || patch
            .terminal_title
            .as_ref()
            .is_some_and(|value| preferences.terminal_title != *value);
    if scalars {
        return true;
    }
    let keymap = patch.keymap.iter().any(|(context, actions)| {
        actions.iter().any(|(action, keys)| {
            let resolved = preferences
                .keymap
                .get(context)
                .and_then(|actions| actions.get(action));
            keys.is_some() && resolved != keys.as_ref()
        })
    });
    let presets = patch
        .presets
        .iter()
        .any(|(slot, preset)| preferences.presets.get(slot) != preset.as_ref());
    keymap || presets
}
