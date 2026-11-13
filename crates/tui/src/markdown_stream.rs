// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A stream's Markdown source, committed a complete line at a time: the
//! controller (`streaming/controller`) renders what each newline completes
//! and leaves the unterminated line to the preview; the stream's end takes
//! the whole source.

/// The source received so far, and how much of it ends with a newline and
/// was handed out.
pub(crate) struct MarkdownStreamCollector {
    buffer: String,
    committed_source_len: usize,
}

impl MarkdownStreamCollector {
    pub(crate) fn new() -> Self {
        Self {
            buffer: String::new(),
            committed_source_len: 0,
        }
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.committed_source_len = 0;
    }

    pub(crate) fn push_delta(&mut self, delta: &str) {
        self.buffer.push_str(delta);
    }

    /// The source completed since the last commit, up to its last newline;
    /// `None` while no newline arrived since, so a block whose meaning the
    /// rest of its line may change is not rendered.
    pub(crate) fn commit_complete_source(&mut self) -> Option<std::ops::Range<usize>> {
        let commit_end = self.buffer.rfind('\n').map(|idx| idx + 1)?;
        let commit_start = self.committed_source_len;
        if commit_end <= commit_start {
            return None;
        }
        self.committed_source_len = commit_end;
        Some(commit_start..commit_end)
    }

    /// The source up to the last newline committed.
    pub(crate) fn committed_source(&self) -> &str {
        &self.buffer[..self.committed_source_len]
    }

    /// The unterminated line, not committed.
    pub(crate) fn pending_source(&self) -> &str {
        &self.buffer[self.committed_source_len..]
    }

    /// The whole source, as it arrived; the collector is cleared.
    pub(crate) fn finalize_and_take_source(&mut self) -> String {
        let out = std::mem::take(&mut self.buffer);
        self.clear();
        out
    }
}

#[cfg(test)]
mod tests;
