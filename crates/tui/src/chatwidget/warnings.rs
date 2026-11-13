// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The warnings the chat keeps for `/warnings` (Codex's
//! `chatwidget/warnings.rs`): its own (a command turned off, a setting of
//! `[tui]` it could not apply), merged with the Agent's when `/warnings`
//! asks for them, without the ones dismissed in this run and without
//! repeats, newest first. At start, one line says how many there are.
//! And a text run as if it were typed (`/presets`, `/config`'s entries).

use super::ChatWidget;
use crate::agent_bridge::{Warning, WarningSource};
use crate::commands::{self, parse_input, ParsedInput};
use crate::history_cell;
use crate::user_message::UserMessage;

impl ChatWidget {
    /// What the interface could not apply at start (`interface`), with the
    /// configuration's warnings (`config`): kept, and counted in one line.
    pub(crate) fn add_startup_warnings(&mut self, interface: Vec<String>, config: Vec<Warning>) {
        self.warnings.extend(
            interface
                .into_iter()
                .map(|text| Warning::new(WarningSource::Interface, text)),
        );
        let count = self.visible_warnings(config).len();
        if count > 0 {
            let noun = if count == 1 { "warning" } else { "warnings" };
            self.add_info_message(format!("{count} {noun}: /warnings"));
        }
    }

    /// `/warnings`: the Agent's `warnings` and the chat's, in the view.
    pub(crate) fn show_warnings(&mut self, warnings: Vec<Warning>) {
        let warnings = self.visible_warnings(warnings);
        self.run_command(&commands::WARNINGS, |context| {
            commands::show_warnings(context, warnings)
        });
    }

    /// Hides the warning with `key` for the rest of the run.
    pub(crate) fn dismiss_warning(&mut self, key: String) {
        self.dismissed_warnings.insert(key);
    }

    /// `agent` and the chat's warnings, but those dismissed, each once (the
    /// latest), newest first.
    fn visible_warnings(&self, agent: Vec<Warning>) -> Vec<Warning> {
        let mut all: Vec<Warning> = self.warnings.iter().cloned().chain(agent).collect();
        all.sort_by_key(|warning| std::cmp::Reverse(warning.at_unix_ms));
        let mut seen = std::collections::HashSet::new();
        all.retain(|warning| {
            let key = warning.key();
            !self.dismissed_warnings.contains(&key) && seen.insert(key)
        });
        all
    }

    /// Runs `text` as if it were typed and sent: the command it names, or
    /// a message; a `/name` naming no command is said so and not sent.
    pub(crate) fn replay_input(&mut self, text: String) {
        match parse_input(&text) {
            ParsedInput::Command(command, args) => {
                if self.bottom_pane.command_disabled(command.name) {
                    self.add_error_message(format!(
                        "'/{}' is turned off: it failed earlier in this run.",
                        command.name
                    ));
                } else {
                    self.dispatch_command(command, args);
                }
            }
            ParsedInput::Unrecognized(name) => {
                let message = format!(
                    r#"Unrecognized command '/{name}'. Type "/" for a list of supported commands."#
                );
                self.add_to_history(history_cell::new_info_event(message, None));
            }
            ParsedInput::Message => self.submit_user_message(UserMessage::from_text(text.trim())),
        }
        self.request_redraw();
    }
}
