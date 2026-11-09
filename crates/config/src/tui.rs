//! `[tui]`: the terminal interface's preferences, read only by the
//! standalone Agent's interface and written by its commands (`/tui`,
//! `/raw`, `/theme`, `/statusline`, `/title`) or `config set tui.<key>`.
//! Each key decodes on its own; an unknown one is kept in the file and
//! ignored. The items of the status line and the terminal title are kept as
//! the ids the interface names them by; it leaves out the ones it does not
//! know.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::fields::{read_object, Fields};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTuiPreferences {
    /// The interface owns the screen and draws the conversation itself
    /// (when the alternate screen is allowed); otherwise the conversation
    /// goes into the terminal's scrollback. Read at launch.
    pub fullscreen_transcript: bool,
    pub alternate_screen: AgentAlternateScreen,
    /// The conversation shown as the plain text it was made from.
    pub raw_output: bool,
    /// The syntax highlighting theme: a bundled theme's name, or a custom
    /// `<name>.tmTheme` in the data directory's `themes/`; `None` for the
    /// default.
    pub theme: Option<String>,
    /// The items of the status line under the composer, in order; empty
    /// for none.
    pub status_line: Vec<String>,
    /// Whether the status line's items take the theme's colors.
    pub status_line_colors: bool,
    /// The items of the terminal's title, in order; empty to leave the
    /// title alone.
    pub terminal_title: Vec<String>,
}

impl Default for AgentTuiPreferences {
    fn default() -> Self {
        Self {
            fullscreen_transcript: true,
            alternate_screen: AgentAlternateScreen::Auto,
            raw_output: false,
            theme: None,
            status_line: DEFAULT_STATUS_LINE.map(String::from).to_vec(),
            status_line_colors: true,
            terminal_title: DEFAULT_TERMINAL_TITLE.map(String::from).to_vec(),
        }
    }
}

/// Codex's defaults.
const DEFAULT_STATUS_LINE: [&str; 3] = ["model-with-reasoning", "current-dir", "session-title"];
const DEFAULT_TERMINAL_TITLE: [&str; 3] = ["activity", "session-title", "project-name"];

/// Whether the interface may use the terminal's alternate screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAlternateScreen {
    /// Yes, but not over SSH.
    #[default]
    Auto,
    Always,
    Never,
}

/// A change to `[tui]`: each field present is written, the others keep
/// what the file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentTuiPreferencesPatch {
    pub fullscreen_transcript: Option<bool>,
    pub alternate_screen: Option<AgentAlternateScreen>,
    pub raw_output: Option<bool>,
    pub theme: Option<String>,
    pub status_line: Option<Vec<String>>,
    pub status_line_colors: Option<bool>,
    pub terminal_title: Option<Vec<String>>,
}

impl AgentTuiPreferencesPatch {
    /// The keys it changes, as the `[tui]` table of `config.toml`.
    pub(crate) fn document(&self) -> Map<String, Value> {
        let mut table = Map::new();
        if let Some(enabled) = self.fullscreen_transcript {
            table.insert("fullscreenTranscript".into(), Value::Bool(enabled));
        }
        if let Some(mode) = self.alternate_screen {
            table.insert("alternateScreen".into(), serde_json::json!(mode));
        }
        if let Some(enabled) = self.raw_output {
            table.insert("rawOutput".into(), Value::Bool(enabled));
        }
        if let Some(theme) = &self.theme {
            table.insert("theme".into(), Value::String(theme.clone()));
        }
        if let Some(items) = &self.status_line {
            table.insert("statusLine".into(), serde_json::json!(items));
        }
        if let Some(enabled) = self.status_line_colors {
            table.insert("statusLineColors".into(), Value::Bool(enabled));
        }
        if let Some(items) = &self.terminal_title {
            table.insert("terminalTitle".into(), serde_json::json!(items));
        }
        Map::from_iter([("tui".to_string(), Value::Object(table))])
    }
}

/// The `[tui]` table of `fields`, each key missing or invalid taking its
/// value in `defaults`.
pub(crate) fn read_tui(
    fields: &mut Fields<'_>,
    defaults: &AgentTuiPreferences,
) -> AgentTuiPreferences {
    read_object(fields, "tui", defaults, |fields, tui| {
        tui.fullscreen_transcript = fields.read("fullscreenTranscript", tui.fullscreen_transcript);
        tui.alternate_screen = fields.read("alternateScreen", tui.alternate_screen);
        tui.raw_output = fields.read("rawOutput", tui.raw_output);
        tui.theme = fields.read("theme", tui.theme.take());
        tui.status_line = fields.read("statusLine", std::mem::take(&mut tui.status_line));
        tui.status_line_colors = fields.read("statusLineColors", tui.status_line_colors);
        tui.terminal_title = fields.read("terminalTitle", std::mem::take(&mut tui.terminal_title));
    })
}

#[cfg(test)]
mod tests;
