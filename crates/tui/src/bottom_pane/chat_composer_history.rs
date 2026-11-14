// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The chat composer's history: shell-style navigation (Up/Down) over the
//! messages sent in this run, then those from earlier sessions, and
//! Ctrl+R's search (Codex's traversal in `chat_composer_history.rs`).
//! Stored messages are read once on the first Up or search. Codex fetches
//! its history file entry by entry; here the stored
//! messages come in one bounded read, so a search waits only while that
//! read runs. An entry is a message with its images (`UserMessage`); a
//! stored one keeps its text only.

use std::collections::HashSet;

use crate::user_message::UserMessage;

/// The navigation state, kept apart from the widget so it is tested alone.
pub(crate) struct ChatComposerHistory {
    /// Messages submitted by the user *during this UI session* (newest at END).
    local_history: Vec<UserMessage>,

    /// What the person sent in the Agent's sessions, newest first.
    stored: StoredHistory,

    /// The search Ctrl+R runs, while it runs.
    search: Option<HistorySearchState>,

    /// Current cursor within the history. `None` indicates the user is *not*
    /// currently browsing history.
    history_cursor: Option<usize>,
    pending_navigation: bool,

    /// The text that was last inserted into the composer as a result of
    /// history navigation. Used to decide if further Up/Down presses should be
    /// treated as navigation versus normal cursor movement.
    last_history_text: Option<String>,
}

impl ChatComposerHistory {
    pub fn new() -> Self {
        Self {
            local_history: Vec::new(),
            stored: StoredHistory::Unread,
            search: None,
            history_cursor: None,
            pending_navigation: false,
            last_history_text: None,
        }
    }

    /// Record a message submitted by the user in the current session so it can
    /// be recalled later.
    pub fn record_local_submission(&mut self, message: UserMessage) {
        if message.text.is_empty() {
            return;
        }

        self.history_cursor = None;
        self.pending_navigation = false;
        self.last_history_text = None;
        self.search = None;

        // Avoid inserting a duplicate if identical to the previous entry.
        if self.local_history.last() == Some(&message) {
            return;
        }

        self.local_history.push(message);
    }

    /// Reset navigation tracking so the next Up key resumes from the latest entry.
    pub fn reset_navigation(&mut self) {
        self.history_cursor = None;
        self.pending_navigation = false;
        self.last_history_text = None;
    }

    /// Should Up/Down key presses be interpreted as history navigation given
    /// the current content and cursor position of `textarea`?
    pub fn should_handle_navigation(&self, text: &str, cursor: usize) -> bool {
        if self.local_history.is_empty()
            && matches!(&self.stored, StoredHistory::Read(entries) if entries.is_empty())
        {
            return false;
        }

        if text.is_empty() {
            return true;
        }

        // Textarea is not empty – only navigate when cursor is at either text boundary and
        // text matches last recalled history entry so regular editing is not
        // hijacked.
        if cursor != 0 && cursor != text.len() {
            return false;
        }

        matches!(&self.last_history_text, Some(prev) if prev == text)
    }

    /// Newest-first offsets remain stable when the older stored entries arrive.
    pub fn navigate_up(&mut self) -> Option<UserMessage> {
        self.search = None;
        let index = self.history_cursor.map_or(0, |index| index + 1);
        let entry = self.populate_history_at_index(index);
        self.pending_navigation = entry.is_none() && matches!(self.stored, StoredHistory::Reading);
        entry
    }

    pub fn navigate_down(&mut self) -> Option<UserMessage> {
        self.cancel_pending_navigation();
        self.search = None;
        match self.history_cursor? {
            0 => {
                self.reset_navigation();
                Some(UserMessage::default())
            }
            index => self.populate_history_at_index(index - 1),
        }
    }

    fn populate_history_at_index(&mut self, index: usize) -> Option<UserMessage> {
        let stored = match &self.stored {
            StoredHistory::Read(entries) => entries.as_slice(),
            StoredHistory::Unread | StoredHistory::Reading => &[],
        };
        // The bounded read may already contain submissions from this run.
        let local_texts: HashSet<&str> = self
            .local_history
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        let entry = self
            .local_history
            .iter()
            .rev()
            .cloned()
            .chain(
                stored
                    .iter()
                    .filter(|text| !text.is_empty() && !local_texts.contains(text.as_str()))
                    .map(|text| UserMessage::stored(text)),
            )
            .nth(index)?
            .available();
        self.history_cursor = Some(index);
        self.last_history_text = Some(entry.text.clone());
        Some(entry)
    }

    pub fn request_stored_read(&mut self) -> bool {
        if !matches!(self.stored, StoredHistory::Unread) {
            return false;
        }
        self.stored = StoredHistory::Reading;
        true
    }

    pub fn cancel_pending_navigation(&mut self) {
        self.pending_navigation = false;
    }

    pub fn resume_navigation(&mut self) -> Option<UserMessage> {
        if !std::mem::take(&mut self.pending_navigation) {
            return None;
        }
        self.navigate_up()
    }

    /// Starts a search; `true` when the stored messages are to be read
    /// now (unless navigation already requested them).
    pub fn begin_search(&mut self) -> bool {
        self.search = None;
        self.cancel_pending_navigation();
        self.request_stored_read()
    }

    /// Clears the search, so the next one starts from the newest match.
    pub fn reset_search(&mut self) {
        self.search = None;
    }

    /// The next step of the search for `query`: a new query, or `restart`,
    /// starts again from the newest match; otherwise the step moves to the
    /// next unique match in `direction`, and stays on the one shown at an
    /// end. A step past the matches found while the stored messages are
    /// still read waits for them ([`HistorySearchResult::Pending`]).
    pub fn search(
        &mut self,
        query: &str,
        direction: HistorySearchDirection,
        restart: bool,
    ) -> HistorySearchResult {
        let reading = matches!(self.stored, StoredHistory::Reading);
        if let Some(search) = self
            .search
            .as_mut()
            .filter(|search| !restart && search.query == query)
        {
            return search.step(direction, reading);
        }
        let matches = self.matches(query);
        self.search
            .insert(HistorySearchState {
                query: query.to_string(),
                matches,
                selected: None,
                waiting: false,
            })
            .step(HistorySearchDirection::Older, reading)
    }

    /// The stored messages, newest first, once read; a search waiting for
    /// them goes on, and its step is returned.
    pub fn on_stored_read(&mut self, entries: Vec<String>) -> Option<HistorySearchResult> {
        self.stored = StoredHistory::Read(entries);
        let query = self.search.as_ref()?.query.clone();
        let matches = self.matches(&query);
        let search = self.search.as_mut()?;
        search.matches = matches;
        search
            .waiting
            .then(|| search.step(HistorySearchDirection::Older, false))
    }

    /// Every entry holding `query` (ignoring case), each text once: this
    /// run's newest first, then the stored ones.
    fn matches(&self, query: &str) -> Vec<UserMessage> {
        let query = query.to_lowercase();
        let stored = match &self.stored {
            StoredHistory::Read(entries) => entries.as_slice(),
            StoredHistory::Unread | StoredHistory::Reading => &[],
        };
        let mut seen = HashSet::new();
        let local = self
            .local_history
            .iter()
            .rev()
            .map(|entry| (&entry.text, Some(entry)));
        local
            .chain(stored.iter().map(|text| (text, None)))
            .filter(|(text, _)| text.to_lowercase().contains(&query))
            .filter(|(text, _)| seen.insert(text.as_str()))
            .map(|(text, local)| local.cloned().unwrap_or_else(|| UserMessage::stored(text)))
            .collect()
    }
}

/// The person's messages in the history store.
enum StoredHistory {
    Unread,
    Reading,
    Read(Vec<String>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistorySearchDirection {
    Older,
    Newer,
}

/// A step of the search. `AtBoundary`: no other match that way, and the one
/// shown stays; `Pending`: none yet, the stored messages are still read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HistorySearchResult {
    Found(UserMessage),
    Pending,
    AtBoundary,
    NotFound,
}

/// One query's matches, newest first, and the one shown.
struct HistorySearchState {
    query: String,
    matches: Vec<UserMessage>,
    selected: Option<usize>,
    /// A step older waits for the stored messages.
    waiting: bool,
}

impl HistorySearchState {
    fn step(&mut self, direction: HistorySearchDirection, reading: bool) -> HistorySearchResult {
        let next = match direction {
            HistorySearchDirection::Older => Some(self.selected.map_or(0, |index| index + 1)),
            HistorySearchDirection::Newer => self.selected.and_then(|index| index.checked_sub(1)),
        };
        self.waiting = false;
        if let Some((index, entry)) =
            next.and_then(|index| Some((index, self.matches.get(index)?.clone())))
        {
            self.selected = Some(index);
            return HistorySearchResult::Found(entry);
        }
        // Older past the matches, or any step before the first, may find
        // one among the stored messages.
        if reading && (direction == HistorySearchDirection::Older || self.selected.is_none()) {
            self.waiting = true;
            return HistorySearchResult::Pending;
        }
        match self.selected {
            Some(_) => HistorySearchResult::AtBoundary,
            None => HistorySearchResult::NotFound,
        }
    }
}

#[cfg(test)]
mod tests;
