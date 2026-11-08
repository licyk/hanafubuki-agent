// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! Shell-style history navigation (Up/Down) in the chat composer, over the
//! messages sent in this session. Codex also reads the persistent history
//! of earlier sessions; the resume picker (5b) is where those come back.

/// The navigation state, kept apart from the widget so it is tested alone.
pub(crate) struct ChatComposerHistory {
    /// Messages submitted by the user *during this UI session* (newest at END).
    local_history: Vec<String>,

    /// Current cursor within the history. `None` indicates the user is *not*
    /// currently browsing history.
    history_cursor: Option<usize>,

    /// The text that was last inserted into the composer as a result of
    /// history navigation. Used to decide if further Up/Down presses should be
    /// treated as navigation versus normal cursor movement.
    last_history_text: Option<String>,
}

impl ChatComposerHistory {
    pub fn new() -> Self {
        Self {
            local_history: Vec::new(),
            history_cursor: None,
            last_history_text: None,
        }
    }

    /// Record a message submitted by the user in the current session so it can
    /// be recalled later.
    pub fn record_local_submission(&mut self, text: String) {
        if text.is_empty() {
            return;
        }

        self.history_cursor = None;
        self.last_history_text = None;

        // Avoid inserting a duplicate if identical to the previous entry.
        if self.local_history.last() == Some(&text) {
            return;
        }

        self.local_history.push(text);
    }

    /// Reset navigation tracking so the next Up key resumes from the latest entry.
    pub fn reset_navigation(&mut self) {
        self.history_cursor = None;
        self.last_history_text = None;
    }

    /// Should Up/Down key presses be interpreted as history navigation given
    /// the current content and cursor position of `textarea`?
    pub fn should_handle_navigation(&self, text: &str, cursor: usize) -> bool {
        if self.local_history.is_empty() {
            return false;
        }

        if text.is_empty() {
            return true;
        }

        // Textarea is not empty – only navigate when cursor is at start and
        // text matches last recalled history entry so regular editing is not
        // hijacked.
        if cursor != 0 {
            return false;
        }

        matches!(&self.last_history_text, Some(prev) if prev == text)
    }

    /// Handle <Up>: the entry to show, if the cursor moved.
    pub fn navigate_up(&mut self) -> Option<String> {
        let total_entries = self.local_history.len();
        if total_entries == 0 {
            return None;
        }

        let next_idx = match self.history_cursor {
            None => total_entries - 1,
            Some(0) => return None, // already at oldest
            Some(idx) => idx - 1,
        };

        self.history_cursor = Some(next_idx);
        self.populate_history_at_index(next_idx)
    }

    /// Handle <Down>.
    pub fn navigate_down(&mut self) -> Option<String> {
        let total_entries = self.local_history.len();
        if total_entries == 0 {
            return None;
        }

        let next_idx_opt = match self.history_cursor {
            None => return None, // not browsing
            Some(idx) if idx + 1 >= total_entries => None,
            Some(idx) => Some(idx + 1),
        };

        match next_idx_opt {
            Some(idx) => {
                self.history_cursor = Some(idx);
                self.populate_history_at_index(idx)
            }
            None => {
                // Past newest – clear and exit browsing mode.
                self.history_cursor = None;
                self.last_history_text = None;
                Some(String::new())
            }
        }
    }

    fn populate_history_at_index(&mut self, idx: usize) -> Option<String> {
        let entry = self.local_history.get(idx).cloned()?;
        self.last_history_text = Some(entry.clone());
        Some(entry)
    }
}

#[cfg(test)]
mod tests;
