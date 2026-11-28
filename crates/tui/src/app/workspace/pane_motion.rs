// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The side pane opens by sliding: the share of it drawn grows to its full
//! width over [`SLIDE_DURATION`], smoothly eased, a frame at a time. The
//! layout asks for the share each frame and gets a frame scheduled while it
//! moves.
//!
//! Closing does not slide. A second conversation's pane is torn down with
//! its transcript, its drafts and its focus the moment it closes, so a slide
//! out would draw an empty pane or hold a conversation that is no longer
//! there; a sidebar panel likewise leaves the screen the moment it is left.

use std::time::{Duration, Instant};

/// How long the pane takes to open or close.
pub(in crate::app) const SLIDE_DURATION: Duration = Duration::from_millis(180);

/// How often the pane is repainted while it slides.
pub(in crate::app) const SLIDE_FRAME: Duration = Duration::from_millis(16);

/// The share of the side pane painted now, between 0 (closed) and 1 (its
/// full width).
#[derive(Clone, Debug, Default)]
pub(in crate::app) struct PaneMotion {
    fraction: f64,
    target: f64,
    slide: Option<(Instant, f64)>,
}

impl PaneMotion {
    /// The share for this frame, `open` being whether the pane should show.
    /// Returns it with whether a frame is still needed: the share keeps
    /// moving until it reaches the target, and then stays.
    pub(in crate::app) fn advance(
        &mut self,
        open: bool,
        now: Instant,
        animate: bool,
    ) -> (f64, bool) {
        let target = f64::from(u8::from(open));
        if !animate || target == 0.0 {
            self.fraction = target;
            self.target = target;
            self.slide = None;
            return (target, false);
        }
        if self.target != target {
            self.target = target;
            self.slide = Some((now, self.fraction));
        }
        let Some((start, from)) = self.slide else {
            return (self.fraction, false);
        };
        let progress =
            now.saturating_duration_since(start).as_secs_f64() / SLIDE_DURATION.as_secs_f64();
        if progress >= 1.0 || from == target {
            self.slide = None;
            self.fraction = target;
            return (target, false);
        }
        let eased = progress * progress * (3.0 - 2.0 * progress);
        self.fraction = from + (target - from) * eased;
        (self.fraction, true)
    }
}

#[cfg(test)]
mod tests;
