// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! Slash command completion and delivery of asynchronous file matches.
//! Mention popup routing and completion live in the sibling mentions module.

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::crossterm::event::KeyModifiers;

use super::ActivePopup;
use super::ChatComposer;
use super::CommandPopup;
use super::InputResult;
use crate::app_event::AppEvent;
use crate::bottom_pane::footer::reset_mode_after_activity;
use crate::commands;
use crate::commands::{parse_input, ParsedInput};
use crate::file_search::FileMatch;
use crate::history_cell;

impl ChatComposer {
    /// Whether a popup or the history search takes the keys.
    pub(crate) fn popup_active(&self) -> bool {
        !matches!(self.active_popup, ActivePopup::None) || self.history_search_active()
    }

    /// Turns `/name` off: the popup leaves it out, and typing it says so.
    pub(crate) fn disable_command(&mut self, name: &'static str) {
        if !self.disabled_commands.contains(&name) {
            self.disabled_commands.push(name);
        }
    }

    pub(crate) fn command_disabled(&self, name: &str) -> bool {
        self.disabled_commands.contains(&name)
    }

    /// Whether `/name` runs here: it is not turned off, and the scope has it.
    pub(crate) fn command_allowed(&self, name: &str) -> bool {
        !self.command_disabled(name) && self.command_scope.is_none_or(|scope| scope.contains(&name))
    }

    /// Limits the commands to `scope` (`None`: every command): the popup
    /// lists only those, and typing another says it is unavailable here.
    pub(crate) fn set_command_scope(&mut self, scope: Option<&'static [&'static str]>) {
        self.command_scope = scope;
    }

    /// The commands the popup leaves out: those turned off, and those
    /// outside the scope.
    fn hidden_commands(&self) -> Vec<&'static str> {
        let mut hidden = self.disabled_commands.clone();
        if let Some(scope) = self.command_scope {
            hidden.extend(
                commands::COMMANDS
                    .into_iter()
                    .map(|command| command.name)
                    .filter(|name| !scope.contains(name)),
            );
        }
        hidden
    }

    pub(crate) fn insert_str(&mut self, text: &str) {
        self.textarea.insert_str(text);
        self.sync_popups();
    }

    /// Integrate results from an asynchronous file search.
    pub(crate) fn on_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        if let ActivePopup::Mention(popup) = &mut self.active_popup {
            popup.set_file_matches(&query, matches);
        }
    }

    /// Handle key event when the slash-command popup is visible.
    pub(super) fn handle_key_event_with_slash_popup(
        &mut self,
        key_event: KeyEvent,
    ) -> (InputResult, bool) {
        self.footer_mode = reset_mode_after_activity(self.footer_mode);
        let ActivePopup::Command(popup) = &mut self.active_popup else {
            unreachable!();
        };

        match key_event {
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                popup.move_up();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                popup.move_down();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                // Dismiss the slash popup; keep the current input untouched.
                self.active_popup = ActivePopup::None;
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                // Ensure popup filtering/selection reflects the latest composer text
                // before applying completion.
                let first_line = self.textarea.text().lines().next().unwrap_or("");
                popup.on_composer_text_change(first_line.to_string());
                if let Some(selected) = popup.selected_item() {
                    let starts_with_cmd = first_line
                        .trim_start()
                        .starts_with(&format!("/{}", selected.name));
                    if !starts_with_cmd {
                        self.textarea
                            .set_text_clearing_elements(&format!("/{} ", selected.name));
                    }
                    if !self.textarea.text().is_empty() {
                        self.textarea.set_cursor(self.textarea.text().len());
                    }
                }
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if let Some(selected) = popup.selected_item() {
                    self.textarea.set_text_clearing_elements("");
                    return (InputResult::Command(selected.command, String::new()), true);
                }
                // Fallback to default newline handling if no command selected.
                self.handle_key_event_without_popup(key_event)
            }
            input => self.handle_input_basic(input),
        }
    }

    /// The command the message names, with the text after its name when
    /// it takes any (Codex's split of `/name args`); `None` when the
    /// message is no command, or text follows a command that takes none.
    /// A command turned off is not run: the transcript says so.
    pub(super) fn try_dispatch_slash_command(&mut self) -> Option<InputResult> {
        let ParsedInput::Command(command, args) = parse_input(self.textarea.text()) else {
            return None;
        };
        self.textarea.set_text_clearing_elements("");
        if self
            .command_scope
            .is_some_and(|scope| !scope.contains(&command.name))
        {
            let message = format!(
                "'/{name}' is unavailable here; Esc returns to the main conversation.",
                name = command.name
            );
            self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                history_cell::new_error_event(message),
            )));
            return Some(InputResult::None);
        }
        if self.command_disabled(command.name) {
            let message = format!(
                "'/{name}' is turned off: it failed earlier in this run.",
                name = command.name
            );
            self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                history_cell::new_error_event(message),
            )));
            return Some(InputResult::None);
        }
        Some(InputResult::Command(command, args))
    }

    /// A message that starts with `/name` naming no command is not sent: the
    /// transcript says so and the text stays. A leading space, or a slash in
    /// the name (a path), sends it as it is.
    pub(super) fn reject_unrecognized_command(&mut self) -> bool {
        let text = self.textarea.text().to_string();
        let ParsedInput::Unrecognized(name) = parse_input(&text) else {
            return false;
        };
        let message = format!(
            r#"Unrecognized command '/{name}'. Type "/" for a list of supported commands."#
        );
        self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
            history_cell::new_info_event(message, None),
        )));
        self.textarea.set_cursor(text.len());
        true
    }

    /// If the cursor is currently within a slash command on the first line,
    /// extract the command name and the rest of the line after it.
    /// Returns None if the cursor is outside a slash command.
    fn slash_command_under_cursor(first_line: &str, cursor: usize) -> Option<(&str, &str)> {
        if !first_line.starts_with('/') {
            return None;
        }

        let name_start = 1usize;
        let name_end = first_line[name_start..]
            .find(char::is_whitespace)
            .map(|idx| name_start + idx)
            .unwrap_or_else(|| first_line.len());

        if cursor > name_end {
            return None;
        }

        let name = &first_line[name_start..name_end];
        let rest_start = first_line[name_end..]
            .find(|c: char| !c.is_whitespace())
            .map(|idx| name_end + idx)
            .unwrap_or(name_end);
        let rest = &first_line[rest_start..];

        Some((name, rest))
    }

    /// Heuristic for whether the typed slash command looks like a valid
    /// prefix for any known command.
    /// Empty names only count when there is no extra content after the '/'.
    fn looks_like_slash_prefix(&self, name: &str, rest_after_name: &str) -> bool {
        if name.is_empty() {
            return rest_after_name.is_empty();
        }
        commands::has_prefix(name, &self.disabled_commands)
    }

    /// Synchronize `self.command_popup` with the current text in the
    /// textarea. This must be called after every modification that can change
    /// the text so the popup is shown/updated/hidden as appropriate.
    pub(super) fn sync_command_popup(&mut self, allow: bool) {
        if !allow {
            if matches!(self.active_popup, ActivePopup::Command(_)) {
                self.active_popup = ActivePopup::None;
            }
            return;
        }
        // Determine whether the caret is inside the initial '/name' token on the first line.
        let text = self.textarea.text();
        let first_line_end = text.find('\n').unwrap_or(text.len());
        let first_line = &text[..first_line_end];
        let cursor = self.textarea.cursor();
        let caret_on_first_line = cursor <= first_line_end;

        let is_editing_slash_command_name = caret_on_first_line
            && Self::slash_command_under_cursor(first_line, cursor)
                .is_some_and(|(name, rest)| self.looks_like_slash_prefix(name, rest));

        match &mut self.active_popup {
            ActivePopup::Command(popup) => {
                if is_editing_slash_command_name {
                    popup.on_composer_text_change(first_line.to_string());
                } else {
                    self.active_popup = ActivePopup::None;
                }
            }
            _ => {
                if is_editing_slash_command_name {
                    let mut command_popup = CommandPopup::new(self.hidden_commands());
                    command_popup.on_composer_text_change(first_line.to_string());
                    self.active_popup = ActivePopup::Command(command_popup);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
