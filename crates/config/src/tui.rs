//! `[tui]`: the terminal interface's preferences, read only by the
//! standalone Agent's interface and written by its commands (`/tui`,
//! `/raw`) or `config set tui.<key>`. Each key decodes on its own; an
//! unknown one is kept in the file and ignored.

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
}

impl Default for AgentTuiPreferences {
    fn default() -> Self {
        Self {
            fullscreen_transcript: true,
            alternate_screen: AgentAlternateScreen::Auto,
            raw_output: false,
        }
    }
}

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
    })
}

#[cfg(test)]
mod tests;
