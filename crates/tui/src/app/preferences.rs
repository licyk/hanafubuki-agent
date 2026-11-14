// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What `/raw` changes: the presentation of the transcript now, saved in
//! `tui.rawOutput`. What the theme, status line and title pages change,
//! saved when their views say so; each of these is written by key, as the
//! settings center writes (`write_setting_values`). The keymap and the
//! presets, which have no spec, are written as a patch of `[tui]`. And the
//! terminal's title, written when it changes and given back at exit.

use std::io::{self, Write};
use std::time::Instant;

use hanafubuki_agent::{
    AgentTuiPreferences, AgentTuiPreferencesPatch, SettingChange, SettingWriteOp, SettingsWrite,
};
use ratatui::backend::Backend;
use serde_json::{json, Value};

use super::App;
use crate::appearance::items::ids;
use crate::appearance::{AppearanceEvent, SPINNER_INTERVAL};
use crate::history_cell::HistoryRenderMode;
use crate::keymap::{self, Keymap};
use crate::pager_overlay::Overlay;
use crate::render::highlight::{
    resolve_theme_by_name, set_syntax_theme, themes_dir, DEFAULT_THEME,
};
use crate::tui::Tui;

impl App {
    /// `/raw`: on, off, or (`None`) the other way.
    pub(super) fn set_raw_output(&mut self, raw: Option<bool>) {
        let raw = raw.unwrap_or(self.render_mode == HistoryRenderMode::Rich);
        self.show_raw_output(raw);
        let state = if raw { "on" } else { "off" };
        let message = if self.mode.is_owned() {
            format!("Raw output {state}.")
        } else {
            format!("Raw output {state} for what follows; the scrollback keeps what it shows.")
        };
        self.write_tui_settings(vec![("tui.rawOutput", json!(raw))], message);
    }

    /// The conversation shown raw, or rich.
    fn show_raw_output(&mut self, raw: bool) {
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
    }

    /// The theme, status line and title pages, and `/statusline` or
    /// `/title` with items: applied, and saved when asked. An update notice
    /// reaches both conversations, a parked one too.
    pub(crate) fn apply_appearance(&mut self, event: AppearanceEvent) {
        let save = match &event {
            AppearanceEvent::ThemeSaved(name) => Some((
                vec![("tui.theme", json!(name))],
                format!("Theme set to {name}."),
            )),
            AppearanceEvent::StatusLine {
                items,
                colors,
                save: true,
            } => {
                let items = items
                    .as_deref()
                    .map(|items| ("tui.statusLine", json!(ids(items))));
                let colors = colors.map(|colors| ("tui.statusLineColors", json!(colors)));
                Some((
                    items.into_iter().chain(colors).collect(),
                    "Status line updated.".to_string(),
                ))
            }
            AppearanceEvent::TerminalTitle { items, save: true } => Some((
                vec![("tui.terminalTitle", json!(ids(items)))],
                "Terminal title updated.".to_string(),
            )),
            AppearanceEvent::StatusLine { save: false, .. }
            | AppearanceEvent::TerminalTitle { save: false, .. }
            | AppearanceEvent::PreviewTerminalTitle(_)
            | AppearanceEvent::UpdateNotice(_) => None,
        };
        if let (AppearanceEvent::UpdateNotice(_), Some(parked)) =
            (&event, self.workspace.parked_mut())
        {
            parked.chat_widget.apply_appearance(event.clone());
        }
        let session_only = matches!(
            event,
            AppearanceEvent::StatusLine { save: false, .. }
                | AppearanceEvent::TerminalTitle { save: false, .. }
        );
        self.chat_widget.apply_appearance(event);
        match save {
            Some((changes, message)) => self.write_tui_settings(changes, message),
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

    /// Writes `[tui]` settings by key off the loop; what they changed is
    /// applied when the write is done ([`Self::tui_settings_written`]). An
    /// empty `message` writes quietly: only a failure is said.
    pub(super) fn write_tui_settings(&mut self, changes: Vec<(&str, Value)>, message: String) {
        let write = SettingsWrite {
            changes: changes
                .into_iter()
                .map(|(key, value)| SettingChange {
                    key: key.to_string(),
                    op: SettingWriteOp::Set(value),
                })
                .collect(),
        };
        self.bridge.write_tui_settings(write, message);
    }

    /// A [`Self::write_tui_settings`] done: `[tui]` taken as it now
    /// resolves, then `message`, or why it was not written.
    pub(super) fn tui_settings_written(
        &mut self,
        mut message: String,
        written: Result<AgentTuiPreferences, String>,
        shadowed: bool,
    ) {
        if message.is_empty() {
            return self.tui_settings_written_quietly(written);
        }
        let reported = match written {
            Ok(preferences) => {
                if shadowed {
                    message.push_str(SHADOWED);
                }
                self.adopt_tui_preferences(preferences);
                self.chat_widget.add_info_message(message.clone());
                Ok(message)
            }
            Err(error) => {
                let error = format!("{message} It was not saved: {error}");
                self.chat_widget.add_error_message(error.clone());
                Err(error)
            }
        };
        self.report_to_settings(reported);
        self.schedule_frame(Instant::now());
    }

    /// A quiet write done: taken as it resolves, or its failure said in the
    /// command center while it is open (its grouping's), else in the
    /// conversation.
    fn tui_settings_written_quietly(&mut self, written: Result<AgentTuiPreferences, String>) {
        match written {
            Ok(preferences) => self.adopt_tui_preferences(preferences),
            Err(error) => {
                let error = format!("The setting was not saved: {error}");
                match self.workspace.agents_mut() {
                    Some(center) => center.set_notice(error),
                    None => self.chat_widget.add_error_message(error),
                }
            }
        }
        self.schedule_frame(Instant::now());
    }

    /// Writes the keymap's or the presets' `patch` into `config.toml` off
    /// the loop; what it changed is applied when the write is done
    /// ([`Self::preferences_saved`]).
    pub(super) fn save(&mut self, patch: AgentTuiPreferencesPatch, message: String) {
        self.bridge.save_tui_preferences(patch, message);
    }

    /// A [`Self::save`] done: what it changed applied, then `message`, or
    /// why it was not saved.
    pub(super) fn preferences_saved(
        &mut self,
        patch: AgentTuiPreferencesPatch,
        mut message: String,
        saved: Result<AgentTuiPreferences, String>,
    ) {
        let reported = match saved {
            Ok(preferences) => {
                if shadowed(&patch, &preferences) {
                    message.push_str(SHADOWED);
                }
                // What the keymap cannot use was kept for `/warnings` at
                // start, and `/keymap` refuses what it would add.
                self.apply_preferences(preferences);
                self.chat_widget.add_info_message(message.clone());
                Ok(message)
            }
            Err(error) => {
                let error = format!("{message} It was not saved: {error}");
                self.chat_widget.add_error_message(error.clone());
                Err(error)
            }
        };
        self.report_to_settings(reported);
        self.schedule_frame(Instant::now());
    }

    /// What a page of the settings center saved, said in the center too
    /// while it is open over the transcript.
    fn report_to_settings(&mut self, outcome: Result<String, String>) {
        if let Some(center) = self.workspace.settings_mut() {
            center.report(outcome);
        }
    }

    /// `[tui]` as a write of the settings center left it, taken whole: the
    /// keymap and the rest [`Self::apply_preferences`] applies, the raw
    /// output, the theme, the status line and the terminal title.
    pub(super) fn adopt_tui_preferences(&mut self, preferences: AgentTuiPreferences) {
        if preferences.raw_output != (self.render_mode == HistoryRenderMode::Raw) {
            self.show_raw_output(preferences.raw_output);
        }
        crate::markdown_render::preferences::set(preferences.rendering);
        let theme = preferences.theme.as_deref().unwrap_or(DEFAULT_THEME);
        if let Some(resolved) = resolve_theme_by_name(theme, themes_dir()) {
            set_syntax_theme(resolved);
        }
        self.chat_widget.adopt_appearance(&preferences);
        self.apply_preferences(preferences);
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

/// What a saved message adds when a layer above the user's file keeps
/// another value.
const SHADOWED: &str =
    " A profile, the project's configuration or -c sets it too, and that one applies.";

/// Whether a keymap entry or a preset `patch` wrote resolves to something
/// else: a layer above the user's file sets it.
pub(super) fn shadowed(
    patch: &AgentTuiPreferencesPatch,
    preferences: &AgentTuiPreferences,
) -> bool {
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
