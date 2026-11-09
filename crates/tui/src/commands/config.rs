// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/config` (Codex's `config_panel/` and `app/config_panel.rs`): the
//! `[tui]` preferences in one panel, toggles and a choice applied and saved
//! at once, and entries that open the other editors (`/providers`, `/mcp`,
//! `/memories`, `/theme`, `/statusline`, `/title`, `/keymap`, `/presets`,
//! `/tui`).

use hanafubuki_agent::{AgentAlternateScreen, AgentTuiPreferencesPatch};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::{CommandContext, CommandSpec};
use crate::app_event::{AppCommand, AppEvent, AppEventSender};
use crate::appearance::AppearanceEvent;
use crate::bottom_pane::{GenericDisplayRow, MenuKey, MenuModel, MenuView};

pub(super) static CONFIG: CommandSpec = CommandSpec {
    name: "config",
    aliases: &[],
    description: "change the interface's settings",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch,
};

/// The editors the panel opens, as the commands that open them.
const ENTRIES: [(&str, &str, &str); 9] = [
    (
        "Providers",
        "/providers",
        "the model Providers and the default model",
    ),
    ("MCP servers", "/mcp", "add, edit and connect MCP servers"),
    (
        "Memories",
        "/memories",
        "use and generate memories, reset them",
    ),
    ("Theme", "/theme", "the syntax highlighting theme"),
    ("Status line", "/statusline", "the items under the composer"),
    (
        "Terminal title",
        "/title",
        "the items of the terminal's title",
    ),
    ("Keyboard", "/keymap", "the keys of every action"),
    ("Presets", "/presets", "inputs kept in slots 0 to 9"),
    ("Interface mode", "/tui", "Fullscreen or Scrollback"),
];

const SCREENS: [AgentAlternateScreen; 3] = [
    AgentAlternateScreen::Auto,
    AgentAlternateScreen::Always,
    AgentAlternateScreen::Never,
];

fn dispatch(context: &mut CommandContext<'_>, _args: &str) {
    let preferences = context.preferences();
    let panel = ConfigPanel {
        raw_output: preferences.raw_output,
        status_line_colors: preferences.status_line_colors,
        fullscreen: preferences.fullscreen_transcript,
        alternate_screen: preferences.alternate_screen,
    };
    let view = MenuView::new(panel, context.app_event_tx());
    context.show_view(Box::new(view));
}

/// The panel's values as last set; each change is sent as it is made.
pub(super) struct ConfigPanel {
    raw_output: bool,
    status_line_colors: bool,
    fullscreen: bool,
    alternate_screen: AgentAlternateScreen,
}

/// The rows before the entries.
const TOGGLES: usize = 4;

impl ConfigPanel {
    fn change(&mut self, row: usize, forward: bool, tx: &AppEventSender) -> MenuKey {
        match row {
            0 => {
                self.raw_output = !self.raw_output;
                tx.send(AppEvent::SetRawOutput(Some(self.raw_output)));
            }
            1 => {
                self.status_line_colors = !self.status_line_colors;
                tx.send(AppEvent::Appearance(AppearanceEvent::StatusLine {
                    items: None,
                    colors: Some(self.status_line_colors),
                    save: true,
                }));
            }
            2 => {
                self.fullscreen = !self.fullscreen;
                tx.send(AppEvent::SetFullscreenTranscript(self.fullscreen));
            }
            3 => {
                let index = SCREENS
                    .iter()
                    .position(|screen| *screen == self.alternate_screen)
                    .unwrap_or_default();
                let step = if forward { 1 } else { SCREENS.len() - 1 };
                self.alternate_screen = SCREENS[(index + step) % SCREENS.len()];
                let patch = AgentTuiPreferencesPatch {
                    alternate_screen: Some(self.alternate_screen),
                    ..AgentTuiPreferencesPatch::default()
                };
                let message = format!(
                    "Saved the alternate screen: {}. It applies at the next launch.",
                    screen_name(self.alternate_screen)
                );
                tx.send(AppEvent::Command(AppCommand::SavePreferences(
                    patch, message,
                )));
            }
            entry => {
                let Some((_, command, _)) = ENTRIES.get(entry - TOGGLES) else {
                    return MenuKey::Ignored;
                };
                tx.send(AppEvent::Command(AppCommand::Replay(command.to_string())));
                return MenuKey::Close;
            }
        }
        MenuKey::Handled
    }
}

impl MenuModel for ConfigPanel {
    fn title(&self) -> String {
        "Settings".to_string()
    }

    fn rows(&self) -> Vec<GenericDisplayRow> {
        let toggle = |on: bool, name: &str, description: &str| GenericDisplayRow {
            name: format!("[{}] {name}", if on { 'x' } else { ' ' }),
            description: Some(description.to_string()),
            ..Default::default()
        };
        let mut rows = vec![
            toggle(
                self.raw_output,
                "Raw output",
                "the conversation as the plain text it was made from",
            ),
            toggle(
                self.status_line_colors,
                "Status line colors",
                "the status line's items in the theme's colors",
            ),
            toggle(
                self.fullscreen,
                "Fullscreen",
                "the interface draws the conversation itself (next launch)",
            ),
            GenericDisplayRow {
                name: format!("Alternate screen: {}", screen_name(self.alternate_screen)),
                description: Some("auto is off over SSH (next launch)".to_string()),
                ..Default::default()
            },
        ];
        rows.extend(
            ENTRIES
                .iter()
                .map(|(name, command, description)| GenericDisplayRow {
                    name: format!("{name} ›"),
                    description: Some(format!("{description} ({command})")),
                    ..Default::default()
                }),
        );
        rows
    }

    fn hints(&self) -> String {
        "space or enter change or open · ←/→ choose · esc close".to_string()
    }

    fn on_key(&mut self, key: KeyEvent, selected: Option<usize>, tx: &AppEventSender) -> MenuKey {
        let Some(row) = selected else {
            return MenuKey::Ignored;
        };
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => self.change(row, true, tx),
            KeyCode::Right if row == TOGGLES - 1 => self.change(row, true, tx),
            KeyCode::Left if row == TOGGLES - 1 => self.change(row, false, tx),
            _ => MenuKey::Ignored,
        }
    }
}

fn screen_name(screen: AgentAlternateScreen) -> &'static str {
    match screen {
        AgentAlternateScreen::Auto => "auto",
        AgentAlternateScreen::Always => "always",
        AgentAlternateScreen::Never => "never",
    }
}
