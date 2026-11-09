// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The slash-command popup: the built-in commands filtered by what follows
//! the slash. Codex's custom prompts and feature-gated commands are not
//! ported.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use super::selection_popup_common::render_rows;
use super::selection_popup_common::GenericDisplayRow;
use crate::render::Insets;
use crate::render::RectExt;
use crate::slash_command::SlashCommand;

// Hide alias commands in the default popup list so each unique action appears once.
// `quit` is an alias of `exit`, so we skip `quit` here.
const ALIAS_COMMANDS: &[SlashCommand] = &[SlashCommand::Quit];

pub(crate) struct CommandPopup {
    command_filter: String,
    state: ScrollState,
}

impl CommandPopup {
    pub(crate) fn new() -> Self {
        Self {
            command_filter: String::new(),
            state: ScrollState::new(),
        }
    }

    /// Update the filter string based on the current composer text. The text
    /// passed in is expected to start with a leading '/'. Everything after the
    /// *first* '/' on the *first* line becomes the active filter that is used
    /// to narrow down the list of available commands.
    pub(crate) fn on_composer_text_change(&mut self, text: String) {
        let first_line = text.lines().next().unwrap_or("");

        if let Some(stripped) = first_line.strip_prefix('/') {
            // Extract the *first* token (sequence of non-whitespace
            // characters) after the slash so that `/clear something` still
            // shows the help for `/clear`.
            let token = stripped.trim_start();
            let cmd_token = token.split_whitespace().next().unwrap_or("");
            self.command_filter = cmd_token.to_string();
        } else {
            // The composer no longer starts with '/'. Reset the filter so the
            // popup shows the *full* command list if it is still displayed
            // for some reason.
            self.command_filter.clear();
        }

        // Reset or clamp selected index based on new filtered list.
        let matches_len = self.filtered_items().len();
        self.state.clamp_selection(matches_len);
        self.state
            .ensure_visible(matches_len, MAX_POPUP_ROWS.min(matches_len));
    }

    /// Determine the preferred height of the popup for a given width.
    /// Accounts for wrapped descriptions so that long tooltips don't overflow.
    pub(crate) fn calculate_required_height(&self, width: u16) -> u16 {
        use super::selection_popup_common::measure_rows_height;
        let rows = self.rows_from_matches(self.filtered());

        measure_rows_height(&rows, &self.state, MAX_POPUP_ROWS, width)
    }

    /// Compute exact/prefix matches over the built-in commands, paired with
    /// optional highlight indices. Preserves the presentation order.
    fn filtered(&self) -> Vec<(SlashCommand, Option<Vec<usize>>)> {
        let filter = self.command_filter.trim();
        if filter.is_empty() {
            return SlashCommand::ALL
                .into_iter()
                .filter(|cmd| !ALIAS_COMMANDS.contains(cmd))
                .map(|cmd| (cmd, None))
                .collect();
        }

        let filter_lower = filter.to_lowercase();
        let filter_chars = filter.chars().count();
        let mut exact = Vec::new();
        let mut prefix = Vec::new();
        for cmd in SlashCommand::ALL {
            let display_lower = cmd.command().to_lowercase();
            let indices = Some((0..filter_chars).collect());
            if display_lower == filter_lower {
                exact.push((cmd, indices));
            } else if display_lower.starts_with(&filter_lower) {
                prefix.push((cmd, indices));
            }
        }
        exact.extend(prefix);
        exact
    }

    fn filtered_items(&self) -> Vec<SlashCommand> {
        self.filtered().into_iter().map(|(c, _)| c).collect()
    }

    fn rows_from_matches(
        &self,
        matches: Vec<(SlashCommand, Option<Vec<usize>>)>,
    ) -> Vec<GenericDisplayRow> {
        matches
            .into_iter()
            .map(|(cmd, indices)| GenericDisplayRow {
                name: format!("/{}", cmd.command()),
                match_indices: indices.map(|v| v.into_iter().map(|i| i + 1).collect()),
                display_shortcut: None,
                description: Some(cmd.description().to_string()),
                wrap_indent: None,
                is_disabled: false,
                disabled_reason: None,
            })
            .collect()
    }

    /// Move the selection cursor one step up.
    pub(crate) fn move_up(&mut self) {
        let len = self.filtered_items().len();
        self.state.move_up_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    /// Move the selection cursor one step down.
    pub(crate) fn move_down(&mut self) {
        let matches_len = self.filtered_items().len();
        self.state.move_down_wrap(matches_len);
        self.state
            .ensure_visible(matches_len, MAX_POPUP_ROWS.min(matches_len));
    }

    /// Return currently selected command, if any.
    pub(crate) fn selected_item(&self) -> Option<SlashCommand> {
        let matches = self.filtered_items();
        self.state
            .selected_idx
            .and_then(|idx| matches.get(idx).copied())
    }
}

impl Widget for &CommandPopup {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let rows = self.rows_from_matches(self.filtered());
        render_rows(
            area.inset(Insets::tlbr(0, 2, 0, 0)),
            buf,
            &rows,
            &self.state,
            MAX_POPUP_ROWS,
            "no matches",
        );
    }
}

#[cfg(test)]
mod tests;
