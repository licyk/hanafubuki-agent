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

use hanafubuki_agent_foundation::settings::{
    SettingApply, SettingChoice, SettingKind, SettingSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::fields::{FieldRead, Fields};
use crate::sections::{described, no_rule, SettingsGroup, SettingsRead, SettingsSection};

const TUI: &str = "tui";

/// The longest input a preset holds, in UTF-8 bytes (Codex's limit).
pub const MAX_PRESET_INPUT_BYTES: usize = 8_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentTuiPreferences {
    /// The interface owns the screen and draws the conversation itself
    /// (when the alternate screen is allowed); otherwise the conversation
    /// goes into the terminal's scrollback. Read at launch.
    pub fullscreen_transcript: bool,
    pub alternate_screen: AgentAlternateScreen,
    /// Whether Scrollback takes the mouse while its tail shows an entry
    /// with a control, so a click reaches it.
    pub scrollback_mouse: AgentScrollbackMouse,
    /// How the command center (`/agents`) groups its sessions; `g` there
    /// changes it.
    pub agents_grouping: AgentAgentsGrouping,
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
    /// How the conversation's Markdown is drawn.
    pub rendering: AgentTuiRendering,
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
            agents_grouping: AgentAgentsGrouping::Project,
            raw_output: false,
            theme: None,
            status_line: DEFAULT_STATUS_LINE.map(String::from).to_vec(),
            status_line_colors: true,
            terminal_title: DEFAULT_TERMINAL_TITLE.map(String::from).to_vec(),
            rendering: AgentTuiRendering::default(),
            keymap: BTreeMap::new(),
            presets: BTreeMap::new(),
        }
    }
}

/// `[tui.rendering]`: the Markdown constructs drawn as the terminal shows
/// them; one turned off is shown as the model wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentTuiRendering {
    /// Pipe tables laid out in columns (or as key/value records when too
    /// wide), tables in a ```` ```md ```` fence included.
    pub tables: bool,
    /// `•` bullets and `☐`/`☑` task-list boxes, for `-` and `[ ]`/`[x]`.
    pub lists: bool,
    /// TeX math between `$…$`, `\(…\)`, `$$…$$` and `\[…\]` in Unicode
    /// notation; off, each formula's source as written.
    pub math: bool,
    /// A ```` ```mermaid ```` fence drawn as a text diagram when its family
    /// and size allow; off, shown as code.
    pub mermaid: bool,
}

impl Default for AgentTuiRendering {
    fn default() -> Self {
        Self {
            tables: true,
            lists: true,
            math: true,
            mermaid: true,
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

/// The headings the command center groups its sessions under (Codex's
/// `agents_overview_grouping`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAgentsGrouping {
    /// The repository a session ran in, its worktrees with it, and the
    /// directory within it; outside a repository, the directory.
    #[default]
    Project,
    Directory,
    Status,
    Model,
}

/// A change to `[tui]`'s keymap and presets, which have no spec: each
/// entry present is written, the others keep what the file holds. The
/// other `[tui]` keys are written by key (`AgentConfig::write_values`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentTuiPreferencesPatch {
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

pub(crate) fn section() -> SettingsSection {
    let choice = |values: &[(&str, &str)]| SettingKind::Choice {
        values: values
            .iter()
            .map(|(value, label)| SettingChoice::new(*value, *label))
            .collect(),
    };
    let items = || SettingKind::TextList {
        maximum_items: None,
        maximum_item_chars: None,
    };
    let at_launch = |spec: SettingSpec| SettingSpec {
        apply: SettingApply::NextLaunch,
        ..spec
    };
    SettingsSection::new(
        TUI,
        "Terminal interface",
        SettingsGroup::Interface,
        10,
        vec![
            at_launch(described(
                SettingSpec::new(
                    "tui.fullscreenTranscript",
                    "Fullscreen",
                    SettingKind::Boolean,
                ),
                "The interface draws the conversation itself; off, it goes into the terminal's scrollback",
            )),
            at_launch(described(
                SettingSpec::new(
                    "tui.alternateScreen",
                    "Alternate screen",
                    choice(&[("auto", "Automatic"), ("always", "Always"), ("never", "Never")]),
                ),
                "Whether the interface may use the alternate screen; automatic does except over SSH",
            )),
            described(
                SettingSpec::new(
                    "tui.scrollbackMouse",
                    "Scrollback mouse",
                    choice(&[("tail", "While the tail has a control"), ("off", "Off")]),
                ),
                "Whether Scrollback captures the mouse while its tail shows a control",
            ),
            described(
                SettingSpec::new(
                    "tui.agentsGrouping",
                    "Command center grouping",
                    choice(&[
                        ("project", "Project"),
                        ("directory", "Directory"),
                        ("status", "Status"),
                        ("model", "Model"),
                    ]),
                ),
                "How the command center (/agents) groups its sessions; g there changes it",
            ),
            described(
                SettingSpec::new("tui.rawOutput", "Raw output", SettingKind::Boolean),
                "The conversation shown as the plain text it was made from",
            ),
            described(
                SettingSpec::new(
                    "tui.theme",
                    "Syntax theme",
                    SettingKind::Text {
                        maximum_chars: None,
                        secret: false,
                        nullable: true,
                    },
                ),
                "The syntax highlighting theme; unset, the default",
            ),
            described(
                SettingSpec::new("tui.statusLine", "Status line", items()),
                "The status line's items, in order",
            ),
            described(
                SettingSpec::new(
                    "tui.statusLineColors",
                    "Status line colors",
                    SettingKind::Boolean,
                ),
                "Whether the status line's items take the theme's colors",
            ),
            described(
                SettingSpec::new("tui.terminalTitle", "Terminal title", items()),
                "The terminal title's items, in order; none leaves the title alone",
            ),
            described(
                SettingSpec::new("tui.rendering.tables", "Tables", SettingKind::Boolean),
                "Pipe tables laid out in columns; off, shown as written",
            ),
            described(
                SettingSpec::new("tui.rendering.lists", "List markers", SettingKind::Boolean),
                "• bullets and ☐/☑ task boxes; off, - and [ ]/[x] as written",
            ),
            described(
                SettingSpec::new("tui.rendering.math", "Math", SettingKind::Boolean),
                "TeX formulas in Unicode notation; off, shown as written",
            ),
            described(
                SettingSpec::new(
                    "tui.rendering.mermaid",
                    "Mermaid diagrams",
                    SettingKind::Boolean,
                ),
                "```mermaid fences drawn as text diagrams; off, shown as code",
            ),
        ],
    )
}

/// The `[tui]` table, each setting read by its spec and each keymap entry
/// and preset on its own, a missing or invalid one taking its value in
/// `defaults`.
pub(crate) fn read_tui(
    read: &mut SettingsRead<'_>,
    defaults: &AgentTuiPreferences,
) -> AgentTuiPreferences {
    let mut tui = read.read(TUI, defaults, no_rule);
    let fields = read.fields();
    let Some(table) = fields.raw(TUI).and_then(Value::as_object) else {
        return tui;
    };
    let path = fields.path(TUI);
    let mut fields = Fields::nested(table, path, fields.repairs());
    read_entries(
        &mut fields,
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
                    FieldRead::Invalid => {
                        fields.invalid(action, Some("not a key or a list of keys; it is ignored"))
                    }
                    FieldRead::Missing => {}
                },
            );
            keymap.retain(|_, actions| !actions.is_empty());
        },
    );
    read_entries(
        &mut fields,
        "presets",
        &mut tui.presets,
        |fields, slot, presets| {
            let preset = fields
                .read_quiet::<AgentTuiPreset>(slot, |preset| preset.validate().map(|()| preset));
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
    tui
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
