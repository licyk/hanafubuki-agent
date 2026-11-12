//! `[tui]`: the terminal interface's preferences, read only by the
//! standalone Agent's interface and written by its commands (`/tui`,
//! `/raw`, `/theme`, `/statusline`, `/title`, `/keymap`, `/presets`,
//! `/config`) or `config set tui.<key>`. Each key decodes on its own; an
//! unknown one is kept in the file and ignored. The items of the status
//! line and the terminal title, and the keymap's contexts, actions and keys,
//! are kept as the strings the interface names them by; it warns of the
//! ones it does not know. The keymap and the presets decode entry by entry
//! and merge over the layer below.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::fields::{read_object, FieldRead, Fields};

/// The longest input a preset holds, in UTF-8 bytes (Codex's limit).
pub const MAX_PRESET_INPUT_BYTES: usize = 8_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTuiPreferences {
    /// The interface owns the screen and draws the conversation itself
    /// (when the alternate screen is allowed); otherwise the conversation
    /// goes into the terminal's scrollback. Read at launch.
    pub fullscreen_transcript: bool,
    pub alternate_screen: AgentAlternateScreen,
    /// Whether Scrollback takes the mouse while its tail shows an entry
    /// with a control, so a click reaches it.
    pub scrollback_mouse: AgentScrollbackMouse,
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
    /// `keymap.<context>.<action>`: the keys of an action, as the
    /// interface writes them (`ctrl-t`, `ctrl-x [`, `leader p`); empty
    /// unbinds it. A layer's entry replaces the one below it.
    pub keymap: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// `presets."0"` to `presets."9"`: an input the interface runs as if it
    /// were typed, a message or a slash command.
    pub presets: BTreeMap<String, AgentTuiPreset>,
}

/// One preset's slot: what it runs, and the name the menu shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTuiPreset {
    pub input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl AgentTuiPreset {
    /// Codex's rules: some text, not a shell command, at most
    /// [`MAX_PRESET_INPUT_BYTES`].
    pub fn validate(&self) -> Result<(), String> {
        if self.input.trim().is_empty() {
            return Err("a preset's input cannot be empty".to_string());
        }
        if self.input.trim_start().starts_with('!') {
            return Err(
                "a preset holds a message or a slash command, not a shell command".to_string(),
            );
        }
        if self.input.len() > MAX_PRESET_INPUT_BYTES {
            return Err(format!(
                "a preset's input cannot exceed {MAX_PRESET_INPUT_BYTES} bytes"
            ));
        }
        Ok(())
    }

    /// Whether `slot` names a preset's slot, `0` to `9`.
    pub fn is_slot(slot: &str) -> bool {
        slot.len() == 1 && slot.as_bytes()[0].is_ascii_digit()
    }
}

impl Default for AgentTuiPreferences {
    fn default() -> Self {
        Self {
            fullscreen_transcript: true,
            alternate_screen: AgentAlternateScreen::Auto,
            scrollback_mouse: AgentScrollbackMouse::Tail,
            raw_output: false,
            theme: None,
            status_line: DEFAULT_STATUS_LINE.map(String::from).to_vec(),
            status_line_colors: true,
            terminal_title: DEFAULT_TERMINAL_TITLE.map(String::from).to_vec(),
            keymap: BTreeMap::new(),
            presets: BTreeMap::new(),
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

/// When Scrollback takes the mouse, which also takes the wheel from the
/// terminal's own scrolling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentScrollbackMouse {
    /// While the tail shows an entry with a control, until the first turn
    /// of the wheel; taken again at the next key or entry.
    #[default]
    Tail,
    /// Never: the keyboard and the transcript only.
    Off,
}

/// A change to `[tui]`: each field present is written, the others keep
/// what the file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentTuiPreferencesPatch {
    pub fullscreen_transcript: Option<bool>,
    pub alternate_screen: Option<AgentAlternateScreen>,
    pub scrollback_mouse: Option<AgentScrollbackMouse>,
    pub raw_output: Option<bool>,
    pub theme: Option<String>,
    pub status_line: Option<Vec<String>>,
    pub status_line_colors: Option<bool>,
    pub terminal_title: Option<Vec<String>>,
    /// Keymap entries to write: `None` removes the entry (the default
    /// applies again), an empty list unbinds the action.
    pub keymap: BTreeMap<String, BTreeMap<String, Option<Vec<String>>>>,
    /// Presets to write: `None` empties the slot.
    pub presets: BTreeMap<String, Option<AgentTuiPreset>>,
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
        if let Some(mouse) = self.scrollback_mouse {
            table.insert("scrollbackMouse".into(), serde_json::json!(mouse));
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
        if !self.keymap.is_empty() {
            let contexts = self.keymap.iter().map(|(context, actions)| {
                let actions = actions
                    .iter()
                    .map(|(action, keys)| (action.clone(), key_value(keys.as_deref())))
                    .collect();
                (context.clone(), Value::Object(actions))
            });
            table.insert("keymap".into(), Value::Object(contexts.collect()));
        }
        if !self.presets.is_empty() {
            let slots = self.presets.iter().map(|(slot, preset)| {
                // A label left out is written as null, so the merge
                // removes the one the file had.
                let value = preset.as_ref().map_or(
                    Value::Null,
                    |preset| serde_json::json!({ "input": preset.input, "label": preset.label }),
                );
                (slot.clone(), value)
            });
            table.insert("presets".into(), Value::Object(slots.collect()));
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
        tui.scrollback_mouse = fields.read("scrollbackMouse", tui.scrollback_mouse);
        tui.raw_output = fields.read("rawOutput", tui.raw_output);
        tui.theme = fields.read("theme", tui.theme.take());
        tui.status_line = fields.read("statusLine", std::mem::take(&mut tui.status_line));
        tui.status_line_colors = fields.read("statusLineColors", tui.status_line_colors);
        tui.terminal_title = fields.read("terminalTitle", std::mem::take(&mut tui.terminal_title));
        read_entries(
            fields,
            "keymap",
            &mut tui.keymap,
            |fields, context, keymap| {
                let below = keymap.entry(context.to_string()).or_default();
                read_entries(
                    fields,
                    context,
                    below,
                    |fields, action, actions| match fields.read_quiet::<KeySpec>(action, Ok) {
                        FieldRead::Valid(keys) => {
                            actions.insert(action.to_string(), keys.into_list());
                        }
                        FieldRead::Invalid => fields
                            .invalid(action, Some("not a key or a list of keys; it is ignored")),
                        FieldRead::Missing => {}
                    },
                );
                keymap.retain(|_, actions| !actions.is_empty());
            },
        );
        read_entries(
            fields,
            "presets",
            &mut tui.presets,
            |fields, slot, presets| {
                let preset = fields.read_quiet::<AgentTuiPreset>(slot, |preset| {
                    preset.validate().map(|()| preset)
                });
                match preset {
                    FieldRead::Valid(preset) if AgentTuiPreset::is_slot(slot) => {
                        presets.insert(slot.to_string(), preset);
                    }
                    FieldRead::Missing => {}
                    _ => fields.invalid(
                        slot,
                        Some("not a preset (a slot 0 to 9 with a valid input); it is ignored"),
                    ),
                }
            },
        );
    })
}

/// One key or a list of them, as `keymap` entries take either.
#[derive(Deserialize)]
#[serde(untagged)]
enum KeySpec {
    One(String),
    Many(Vec<String>),
}

impl KeySpec {
    fn into_list(self) -> Vec<String> {
        match self {
            Self::One(key) => vec![key],
            Self::Many(keys) => keys,
        }
    }
}

/// The keys of an entry as the file holds them: one key as a string, the
/// others as a list, `None` as null (removed).
fn key_value(keys: Option<&[String]>) -> Value {
    match keys {
        None => Value::Null,
        Some([key]) => Value::String(key.clone()),
        Some(keys) => serde_json::json!(keys),
    }
}

/// Reads each entry of the table at `field` with `decode`, over `target`
/// (what the layer below holds); a field that is not a table is left out.
fn read_entries<T>(
    fields: &mut Fields<'_>,
    field: &str,
    target: &mut T,
    mut decode: impl FnMut(&mut Fields<'_>, &str, &mut T),
) {
    let Some(raw) = fields.raw(field) else {
        return;
    };
    let Some(object) = raw.as_object() else {
        fields.invalid(field, Some("not a table; it is ignored"));
        return;
    };
    let path = fields.path(field);
    let mut nested = Fields::nested(object, path, fields.repairs());
    for key in object.keys() {
        decode(&mut nested, key, target);
    }
}

#[cfg(test)]
mod tests;
