/// Upper bound on a published reasoning headline. A summary line that streams
/// past it keeps its bounded prefix; the remainder of that line is dropped.
pub(crate) const MAX_REASONING_HEADLINE_BYTES: usize = 200;

/// Projects a streamed reasoning summary onto the single line that best says
/// what the model is doing right now.
///
/// The headline is the latest non-empty summary line, including the line that
/// is still streaming, with Markdown heading markers and a leading bold span
/// removed. A bold span that has not closed yet yields no headline, so the
/// previous one stays visible until the heading is complete. Summary parts are
/// paragraphs of their own: a new part ends the streaming line even though the
/// provider sends no newline between parts.
///
/// Only streamed fragments feed the headline; a summary that arrives whole on
/// item completion is not presented. Only the streaming line and the headline
/// are retained, both bounded, so the projection never grows with the
/// reasoning it observes.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReasoningHeadlineProjection {
    summary_index: Option<u32>,
    line: String,
    headline: Option<String>,
}

impl ReasoningHeadlineProjection {
    /// Feeds a streamed summary fragment and returns the headline when it changed.
    pub(crate) fn push(&mut self, summary_index: u32, delta: &str) -> Option<String> {
        let previous = self.headline.clone();
        if self.summary_index.replace(summary_index) != Some(summary_index) {
            self.finish_line();
        }
        let mut segments = delta.split('\n').peekable();
        while let Some(segment) = segments.next() {
            self.append_bounded(segment);
            if segments.peek().is_some() {
                self.finish_line();
            }
        }
        if let Some(candidate) = headline_from_line(&self.line) {
            self.headline = Some(candidate);
        }
        (self.headline != previous)
            .then(|| self.headline.clone())
            .flatten()
    }

    fn finish_line(&mut self) {
        if let Some(candidate) = headline_from_line(&self.line) {
            self.headline = Some(candidate);
        }
        self.line.clear();
    }

    fn append_bounded(&mut self, segment: &str) {
        let room = MAX_REASONING_HEADLINE_BYTES.saturating_sub(self.line.len());
        if segment.len() <= room {
            self.line.push_str(segment);
            return;
        }
        let mut cut = room;
        while cut > 0 && !segment.is_char_boundary(cut) {
            cut -= 1;
        }
        self.line.push_str(&segment[..cut]);
    }
}

/// The presentable text of one summary line, or `None` when the line has
/// nothing to show yet.
fn headline_from_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("<!--") {
        return None;
    }
    let line = line.trim_start_matches('#').trim();
    let headline = match line.strip_prefix("**") {
        Some(stripped) => {
            let (bold, trailing) = stripped.split_once("**")?;
            format!("{bold}{trailing}")
        }
        None => line.to_string(),
    };
    (!headline.is_empty()).then_some(headline)
}

#[cfg(test)]
mod tests;
