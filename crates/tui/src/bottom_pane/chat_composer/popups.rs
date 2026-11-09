// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The composer's popups: the slash commands while the first line's
//! `/name` token is edited, and the file search while the caret is on an
//! `@token`. Which one shows is decided after every key (`sync_popups`);
//! while one shows, it takes the navigation and completion keys.

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::crossterm::event::KeyModifiers;

use super::ActivePopup;
use super::ChatComposer;
use super::CommandPopup;
use super::FileSearchPopup;
use super::InputResult;
use crate::app_event::AppEvent;
use crate::bottom_pane::footer::reset_mode_after_activity;
use crate::bottom_pane::textarea::TextArea;
use crate::commands;
use crate::file_search::FileMatch;
use crate::history_cell;

impl ChatComposer {
    /// Return true if either the slash-command popup or the file-search popup is active.
    pub(crate) fn popup_active(&self) -> bool {
        !matches!(self.active_popup, ActivePopup::None)
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

    pub(crate) fn insert_str(&mut self, text: &str) {
        self.textarea.insert_str(text);
        self.sync_popups();
    }

    /// Integrate results from an asynchronous file search.
    pub(crate) fn on_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        // Only apply if user is still editing a token starting with `query`.
        let Some(current_token) = Self::current_at_token(&self.textarea) else {
            return;
        };
        if !current_token.starts_with(&query) {
            return;
        }
        if let ActivePopup::File(popup) = &mut self.active_popup {
            popup.set_matches(&query, matches);
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

    /// Handle key events when file search popup is visible.
    pub(super) fn handle_key_event_with_file_popup(
        &mut self,
        key_event: KeyEvent,
    ) -> (InputResult, bool) {
        self.footer_mode = reset_mode_after_activity(self.footer_mode);
        let ActivePopup::File(popup) = &mut self.active_popup else {
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
                // Hide popup without modifying text, remember token to avoid immediate reopen.
                if let Some(tok) = Self::current_at_token(&self.textarea) {
                    self.dismissed_file_popup_token = Some(tok);
                }
                self.active_popup = ActivePopup::None;
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if let Some(mention) = popup.selected_match().map(FileMatch::mention) {
                    self.insert_selected_path(&mention);
                }
                // No selection: treat Enter as closing the popup/session.
                self.active_popup = ActivePopup::None;
                (InputResult::None, true)
            }
            input => self.handle_input_basic(input),
        }
    }

    /// The command the message names, with the text after its name when
    /// it takes any (Codex's split of `/name args`); `None` when the
    /// message is no command, or text follows a command that takes none.
    /// A command turned off is not run: the transcript says so.
    pub(super) fn try_dispatch_slash_command(&mut self) -> Option<InputResult> {
        let text = self.textarea.text();
        let (name, rest) = parse_slash_name(text.lines().next().unwrap_or(""))?;
        let command = commands::find(name)?;
        let args = match (command.inline_args, rest.is_empty()) {
            (true, _) => parse_slash_name(text)?.1.trim_end().to_string(),
            (false, true) => String::new(),
            (false, false) => return None,
        };
        self.textarea.set_text_clearing_elements("");
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
        let Some((name, _rest)) = parse_slash_name(&text) else {
            return false;
        };
        if name.contains('/') || commands::find(name).is_some() {
            return false;
        }
        let message = format!(
            r#"Unrecognized command '/{name}'. Type "/" for a list of supported commands."#
        );
        self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
            history_cell::new_info_event(message, None),
        )));
        self.textarea.set_cursor(text.len());
        true
    }

    pub(super) fn sync_popups(&mut self) {
        let file_token = Self::current_at_token(&self.textarea);
        let browsing_history = self
            .history
            .should_handle_navigation(self.textarea.text(), self.textarea.cursor());
        // When browsing input history (shell-style Up/Down recall), skip all popup
        // synchronization so nothing steals focus from continued history navigation.
        if browsing_history {
            self.end_file_search();
            self.active_popup = ActivePopup::None;
            return;
        }

        self.sync_command_popup(file_token.is_none());

        if matches!(self.active_popup, ActivePopup::Command(_)) {
            self.end_file_search();
            self.dismissed_file_popup_token = None;
            return;
        }

        if let Some(token) = file_token {
            self.sync_file_search_popup(token);
            return;
        }

        self.end_file_search();
        self.dismissed_file_popup_token = None;
        if matches!(self.active_popup, ActivePopup::File(_)) {
            self.active_popup = ActivePopup::None;
        }
    }

    fn end_file_search(&mut self) {
        if self.current_file_query.take().is_some() {
            self.app_event_tx
                .send(AppEvent::StartFileSearch(String::new()));
        }
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
    fn sync_command_popup(&mut self, allow: bool) {
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
                    let mut command_popup = CommandPopup::new(self.disabled_commands.clone());
                    command_popup.on_composer_text_change(first_line.to_string());
                    self.active_popup = ActivePopup::Command(command_popup);
                }
            }
        }
    }

    /// Synchronize `self.file_search_popup` with the current text in the textarea.
    /// Note this is only called when self.active_popup is NOT Command.
    fn sync_file_search_popup(&mut self, query: String) {
        // If user dismissed popup for this exact query, don't reopen until text changes.
        if self.dismissed_file_popup_token.as_ref() == Some(&query) {
            return;
        }

        self.app_event_tx
            .send(AppEvent::StartFileSearch(query.clone()));

        match &mut self.active_popup {
            ActivePopup::File(popup) => {
                if query.is_empty() {
                    popup.set_empty_prompt();
                } else {
                    popup.set_query(&query);
                }
            }
            _ => {
                let mut popup = FileSearchPopup::new();
                if query.is_empty() {
                    popup.set_empty_prompt();
                } else {
                    popup.set_query(&query);
                }
                self.active_popup = ActivePopup::File(popup);
            }
        }

        self.current_file_query = (!query.is_empty()).then_some(query);
        self.dismissed_file_popup_token = None;
    }

    /// Extract a token prefixed with `prefix` under the cursor, if any.
    ///
    /// The returned string **does not** include the prefix.
    ///
    /// Behavior:
    /// - The cursor may be anywhere *inside* the token (including on the
    ///   leading prefix). It does **not** need to be at the end of the line.
    /// - A token is delimited by ASCII whitespace (space, tab, newline).
    /// - If the token under the cursor starts with `prefix`, that token is
    ///   returned without the leading prefix. A lone prefix character before
    ///   the cursor yields `Some(String::new())` to surface hints, unless
    ///   whitespace follows the cursor.
    fn current_prefixed_token(textarea: &TextArea, prefix: char) -> Option<String> {
        let cursor_offset = textarea.cursor();
        let text = textarea.text();

        // Adjust the provided byte offset to the nearest valid char boundary at or before it.
        let safe_cursor = Self::clamp_to_char_boundary(text, cursor_offset);

        // Split the line around the (now safe) cursor position.
        let before_cursor = &text[..safe_cursor];
        let after_cursor = &text[safe_cursor..];

        // Detect whether we're on whitespace at the cursor boundary.
        let at_whitespace = if safe_cursor < text.len() {
            text[safe_cursor..]
                .chars()
                .next()
                .map(char::is_whitespace)
                .unwrap_or(false)
        } else {
            false
        };

        // Left candidate: token containing the cursor position.
        let start_left = before_cursor
            .char_indices()
            .rfind(|(_, c)| c.is_whitespace())
            .map(|(idx, c)| idx + c.len_utf8())
            .unwrap_or(0);
        let end_left_rel = after_cursor
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(idx, _)| idx)
            .unwrap_or(after_cursor.len());
        let end_left = safe_cursor + end_left_rel;
        let token_left = if start_left < end_left {
            Some(&text[start_left..end_left])
        } else {
            None
        };

        // Right candidate: token immediately after any whitespace from the cursor.
        let ws_len_right: usize = after_cursor
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(char::len_utf8)
            .sum();
        let start_right = safe_cursor + ws_len_right;
        let end_right_rel = text[start_right..]
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(idx, _)| idx)
            .unwrap_or(text.len() - start_right);
        let end_right = start_right + end_right_rel;
        let token_right = if start_right < end_right {
            Some(&text[start_right..end_right])
        } else {
            None
        };

        let left_match = token_left.filter(|t| t.starts_with(prefix));
        let right_match = token_right.filter(|t| t.starts_with(prefix));

        let left_prefixed = left_match.map(|t| t[prefix.len_utf8()..].to_string());
        let right_prefixed = right_match.map(|t| t[prefix.len_utf8()..].to_string());

        if at_whitespace {
            if right_prefixed.is_some() {
                return right_prefixed;
            }
            if token_left.is_some_and(|t| t == prefix.to_string()) {
                return None;
            }
            return left_prefixed;
        }
        if after_cursor.starts_with(prefix) {
            return right_prefixed.or(left_prefixed);
        }
        left_prefixed.or(right_prefixed)
    }

    /// Extract the `@token` that the cursor is currently positioned on, if any.
    ///
    /// The returned string **does not** include the leading `@`.
    pub(super) fn current_at_token(textarea: &TextArea) -> Option<String> {
        Self::current_prefixed_token(textarea, '@')
    }

    /// Replace the active `@token` (the one under the cursor) with `path`.
    ///
    /// The algorithm mirrors `current_at_token` so replacement works no matter
    /// where the cursor is within the token and regardless of how many
    /// `@tokens` exist in the line.
    fn insert_selected_path(&mut self, path: &str) {
        let cursor_offset = self.textarea.cursor();
        let text = self.textarea.text();
        // Clamp to a valid char boundary to avoid panics when slicing.
        let safe_cursor = Self::clamp_to_char_boundary(text, cursor_offset);

        let before_cursor = &text[..safe_cursor];
        let after_cursor = &text[safe_cursor..];

        // Determine token boundaries.
        let start_idx = before_cursor
            .char_indices()
            .rfind(|(_, c)| c.is_whitespace())
            .map(|(idx, c)| idx + c.len_utf8())
            .unwrap_or(0);

        let end_rel_idx = after_cursor
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(idx, _)| idx)
            .unwrap_or(after_cursor.len());
        let end_idx = safe_cursor + end_rel_idx;

        // If the path contains whitespace, wrap it in double quotes so it
        // reads as one path. Avoid adding quotes when the path already
        // contains one to keep behavior simple.
        let needs_quotes = path.chars().any(char::is_whitespace);
        let inserted = if needs_quotes && !path.contains('"') {
            format!("\"{path}\"")
        } else {
            path.to_string()
        };

        // Replace the slice `[start_idx, end_idx)` with the chosen path and a trailing space.
        let mut new_text =
            String::with_capacity(text.len() - (end_idx - start_idx) + inserted.len() + 1);
        new_text.push_str(&text[..start_idx]);
        new_text.push_str(&inserted);
        new_text.push(' ');
        new_text.push_str(&text[end_idx..]);

        // Path replacement is plain text; rebuild without carrying elements.
        self.textarea.set_text_clearing_elements(&new_text);
        let new_cursor = start_idx.saturating_add(inserted.len()).saturating_add(1);
        self.textarea.set_cursor(new_cursor);
    }
}

/// The `/name` a line starts with and what follows it, trimmed at the start.
fn parse_slash_name(line: &str) -> Option<(&str, &str)> {
    let stripped = line.strip_prefix('/')?;
    let name_end = stripped
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(stripped.len(), |(idx, _)| idx);
    let name = &stripped[..name_end];
    if name.is_empty() {
        return None;
    }
    Some((name, stripped[name_end..].trim_start()))
}

#[cfg(test)]
mod tests;
