//! The line a view shows while it waits for something that can take
//! seconds: the running bullet, what it waits for, and from the first
//! second on how long it has waited (`• Listing models… · 3s`).

use std::time::{Duration, Instant};

use ratatui::style::Stylize;
use ratatui::text::Line;

use crate::shimmer::spinner;
use crate::status_indicator_widget::fmt_elapsed_compact;
use crate::tui::FrameRequester;

/// How soon after a drawing the next is asked for: the bullet turns every
/// 600 ms and the count every second. No shimmer, which would redraw the
/// view 30 times a second.
const NEXT_FRAME: Duration = Duration::from_millis(200);

pub(crate) struct LoadingLine {
    frames: FrameRequester,
    animations: bool,
    start: Instant,
}

impl LoadingLine {
    pub(crate) fn new(frames: FrameRequester, animations: bool) -> Self {
        Self {
            frames,
            animations,
            start: Instant::now(),
        }
    }

    /// Still, for a snapshot: a dim bullet, and its start an hour ahead, so
    /// the count stays under a second however slowly the test runs.
    #[cfg(test)]
    pub(crate) fn still() -> Self {
        Self {
            frames: FrameRequester::test_dummy(),
            animations: false,
            start: Instant::now() + Duration::from_secs(3600),
        }
    }

    /// Counts from now again.
    pub(crate) fn restart(&mut self) {
        self.start = Instant::now();
    }

    /// The line saying `text`; drawing it asks for the next frame, so the
    /// line moves while it is drawn and nothing is asked once it is not.
    pub(crate) fn line(&self, text: &str) -> Line<'static> {
        self.frames.schedule_frame_in(NEXT_FRAME);
        let elapsed = self.start.elapsed().as_secs();
        let mut spans = vec![
            spinner(Some(self.start), self.animations),
            " ".into(),
            text.to_string().into(),
        ];
        if elapsed > 0 {
            spans.push(format!(" · {}", fmt_elapsed_compact(elapsed)).dim());
        }
        Line::from(spans)
    }
}

#[cfg(test)]
mod tests;
