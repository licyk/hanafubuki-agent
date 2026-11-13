// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Ctrl+R's reverse search through the messages sent (Codex's
//! `chat_composer/history_search.rs`): the footer's row becomes the query,
//! and the composer previews the match in place of the draft, put aside
//! (`draft`) until the search ends.
//!
//! The search opens idle, previewing nothing. Typing or pasting edits the
//! query and starts again from the newest match; the search's key
//! (`composer.history_search`, Ctrl+R) and Up step to an older unique
//! match, Ctrl+S and Down to a newer one, staying on the one shown at an
//! end. Enter takes the match as a draft to edit; Esc (or Ctrl+C, which
//! the bottom pane hands here) puts the draft back. The matches are this
//! run's messages, then those of the Agent's sessions, which the first
//! search of the composer has read (`AppEvent::ReadRecentMessages`); until
//! they come, a search past this run's matches says `searching`.
//!
//! Codex's Vim states, its sparkles and its frozen fallback preview are not
//! ported.

mod draft;

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{ChatComposer, InputResult};
use crate::app_event::AppEvent;
use crate::bottom_pane::chat_composer_history::{HistorySearchDirection, HistorySearchResult};
use crate::bottom_pane::footer::reset_mode_after_activity;
use crate::key_hint::{self, has_ctrl_or_alt};
use crate::keymap::{self, Action};
use crate::ui_consts::FOOTER_INDENT_COLS;
use draft::SavedDraft;

const PROMPT: &str = "reverse-i-search: ";

/// One search, from Ctrl+R to Enter or Esc.
pub(super) struct HistorySearchSession {
    /// The draft, put aside while a match shows in its place.
    saved: Option<SavedDraft>,
    query: String,
    status: HistorySearchStatus,
}

/// What the footer's row says after the query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistorySearchStatus {
    Idle,
    Searching,
    Match,
    NoMatch,
}

impl ChatComposer {
    pub(crate) fn history_search_active(&self) -> bool {
        self.history_search.is_some()
    }

    /// Whether `key` is the search's: one that opens it, or any while it is
    /// open.
    pub(super) fn takes_history_search_key(&self, key: &KeyEvent) -> bool {
        self.history_search.is_some()
            || (key.kind == KeyEventKind::Press && keymap::matches(Action::HistorySearch, key))
    }

    /// Opens the search, or takes a key of the open one: plain characters
    /// edit the query, the steps move between the matches, Enter takes the
    /// one shown and Esc puts the draft back.
    pub(super) fn handle_history_search_key(&mut self, key: KeyEvent) -> (InputResult, bool) {
        if self.history_search.is_none() {
            self.begin_history_search();
            return (InputResult::None, true);
        }
        if key.kind == KeyEventKind::Release {
            return (InputResult::None, false);
        }
        let ctrl = |c: char| key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char(c);
        if keymap::matches(Action::HistorySearch, &key) || key.code == KeyCode::Up {
            self.history_search_in_direction(HistorySearchDirection::Older);
        } else if ctrl('s') || key.code == KeyCode::Down {
            self.history_search_in_direction(HistorySearchDirection::Newer);
        } else if key.code == KeyCode::Esc || key.code == KeyCode::Char('\u{3}') {
            self.cancel_history_search();
        } else if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE {
            self.accept_history_search();
        } else if key.code == KeyCode::Backspace || ctrl('h') {
            self.update_history_search_query(|query| {
                query.pop();
            });
        } else if ctrl('u') {
            self.update_history_search_query(String::clear);
        } else if let KeyCode::Char(c) = key.code {
            if !has_ctrl_or_alt(key.modifiers) {
                self.update_history_search_query(|query| query.push(c));
            }
        }
        (InputResult::None, true)
    }

    /// Puts the draft back and closes the search; `false` when none was
    /// open.
    pub(crate) fn cancel_history_search(&mut self) -> bool {
        if self.history_search.is_none() {
            return false;
        }
        self.show_saved_draft();
        self.end_history_search();
        self.history.reset_navigation();
        true
    }

    /// A paste goes to the open search's query.
    pub(super) fn paste_into_history_search(&mut self, pasted: &str) -> bool {
        if self.history_search.is_none() {
            return false;
        }
        if !pasted.is_empty() {
            self.update_history_search_query(|query| query.push_str(pasted));
        }
        true
    }

    /// What the person sent in the Agent's sessions, newest first, read
    /// for the search; one waiting for them goes on.
    pub(crate) fn on_recent_messages(&mut self, messages: Vec<String>) {
        if let Some(result) = self.history.on_stored_read(messages) {
            self.apply_history_search_result(result);
        }
    }

    /// The footer's row while the search is open: the query, then what it
    /// found.
    pub(super) fn history_search_line(&self) -> Option<Line<'static>> {
        let search = self.history_search.as_ref()?;
        let mut line = Line::from(vec![PROMPT.dim(), display_query(&search.query).cyan()]);
        match search.status {
            HistorySearchStatus::Idle => {}
            HistorySearchStatus::Searching => line.push_span("  searching".dim()),
            HistorySearchStatus::Match => {
                line.push_span("  ".dim());
                line.push_span(key_hint::plain(KeyCode::Enter));
                line.push_span(" accept".dim());
                line.push_span(" · ".dim());
                line.push_span(key_hint::plain(KeyCode::Esc));
                line.push_span(" cancel".dim());
            }
            HistorySearchStatus::NoMatch => line.push_span("  no match".red()),
        }
        Some(line)
    }

    /// The cursor at the query's end in the footer's row `area`, kept
    /// inside it.
    pub(super) fn history_search_cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let search = self.history_search.as_ref()?;
        if area.is_empty() {
            return None;
        }
        let indent = (FOOTER_INDENT_COLS as u16).min(area.width.saturating_sub(1));
        let typed = Line::from(format!("{PROMPT}{}", display_query(&search.query))).width();
        let x = area
            .x
            .saturating_add(indent)
            .saturating_add(u16::try_from(typed).unwrap_or(u16::MAX));
        Some((x.min(area.right().saturating_sub(1)), area.y))
    }

    fn begin_history_search(&mut self) {
        if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
            self.handle_paste(pasted);
        }
        self.paste_burst.clear_window_after_non_char();
        self.history_search = Some(HistorySearchSession {
            saved: None,
            query: String::new(),
            status: HistorySearchStatus::Idle,
        });
        self.footer_mode = reset_mode_after_activity(self.footer_mode);
        if self.history.begin_search() {
            self.app_event_tx.send(AppEvent::ReadRecentMessages);
        }
    }

    fn history_search_in_direction(&mut self, direction: HistorySearchDirection) {
        let Some(query) = self.history_search.as_ref().map(|s| s.query.clone()) else {
            return;
        };
        if query.is_empty() {
            return;
        }
        let result = self.history.search(&query, direction, false);
        self.apply_history_search_result(result);
    }

    /// Edits the query and starts again from the newest match; an empty
    /// one shows the draft and waits.
    fn update_history_search_query(&mut self, edit: impl FnOnce(&mut String)) {
        let Some(search) = self.history_search.as_mut() else {
            return;
        };
        edit(&mut search.query);
        let query = search.query.clone();
        self.show_saved_draft();
        if query.is_empty() {
            self.history.reset_search();
            self.set_history_search_status(HistorySearchStatus::Idle);
            return;
        }
        let result = self
            .history
            .search(&query, HistorySearchDirection::Older, true);
        self.apply_history_search_result(result);
    }

    /// Shows a step's result: a match in the draft's place; none, the
    /// draft. At an end the match shown stays.
    fn apply_history_search_result(&mut self, result: HistorySearchResult) {
        let Some(query) = self.history_search.as_ref().map(|s| s.query.clone()) else {
            return;
        };
        let status = match result {
            HistorySearchResult::Found(entry) => {
                self.show_match(&entry, match_ranges(&entry, &query));
                HistorySearchStatus::Match
            }
            HistorySearchResult::Pending => HistorySearchStatus::Searching,
            HistorySearchResult::AtBoundary => HistorySearchStatus::Match,
            HistorySearchResult::NotFound => {
                self.show_saved_draft();
                HistorySearchStatus::NoMatch
            }
        };
        self.set_history_search_status(status);
    }

    /// Enter: the match shown becomes the draft, to edit; with none,
    /// nothing happens.
    fn accept_history_search(&mut self) {
        let matched = self
            .history_search
            .as_ref()
            .is_some_and(|search| search.status == HistorySearchStatus::Match);
        if matched {
            self.end_history_search();
            self.textarea.set_highlights(Vec::new());
            self.textarea.set_cursor(self.textarea.text().len());
        }
    }

    fn end_history_search(&mut self) {
        self.history_search = None;
        self.history.reset_search();
        self.footer_mode = reset_mode_after_activity(self.footer_mode);
    }

    fn set_history_search_status(&mut self, status: HistorySearchStatus) {
        if let Some(search) = self.history_search.as_mut() {
            search.status = status;
        }
    }
}

/// The query as the footer shows it: a line break as `↵`, a tab as `⇥`.
fn display_query(query: &str) -> String {
    query.replace('\n', "↵").replace('\t', "⇥")
}

/// Where `query` occurs in `text`, ignoring case, as byte ranges of `text`
/// (a character whose lower case is longer is matched whole).
fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query: String = query.chars().flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Vec::new();
    }
    // The lower-cased text, and for each of its characters the bytes of the
    // character of `text` it came from.
    let mut folded = String::new();
    let mut origins: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    for (start, c) in text.char_indices() {
        let original = start..start + c.len_utf8();
        for lower in c.to_lowercase() {
            let at = folded.len();
            folded.push(lower);
            origins.push((at..folded.len(), original.clone()));
        }
    }
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(found) = folded[from..].find(&query) {
        let start = from + found;
        let end = start + query.len();
        let first = origins.iter().find(|(folded, _)| folded.end > start);
        let last = origins.iter().rev().find(|(folded, _)| folded.start < end);
        if let (Some((_, first)), Some((_, last))) = (first, last) {
            ranges.push(first.start..last.end);
        }
        from = end;
    }
    ranges
}

#[cfg(test)]
mod tests;
