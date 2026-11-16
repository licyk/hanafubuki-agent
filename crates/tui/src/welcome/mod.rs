//! Fresh-conversation decoration, inspired by Codex's empty-state animation.
//! It is painted into the viewport, never inserted into conversation history.

mod renderer;

use std::time::{Duration, Instant};

use ratatui::{buffer::Buffer, layout::Rect};

use renderer::Renderer;

pub(crate) const FRAME_INTERVAL: Duration = Duration::from_millis(83);
const FADE_DURATION: f32 = 0.45;

pub(crate) struct Welcome {
    eligible: bool,
    opacity: f32,
    elapsed: Duration,
    last_frame: Option<Instant>,
    renderer: Renderer,
}

impl Welcome {
    pub(crate) fn new(eligible: bool) -> Self {
        Self {
            eligible,
            opacity: 0.0,
            elapsed: Duration::ZERO,
            last_frame: None,
            renderer: Renderer::default(),
        }
    }

    pub(crate) fn dismiss(&mut self) {
        self.eligible = false;
    }

    pub(crate) fn needs_space(&self) -> bool {
        self.eligible || self.opacity > 0.0
    }

    pub(crate) fn hide(&mut self) {
        self.opacity = 0.0;
        self.last_frame = None;
    }

    /// The empty region above the composer; existing headers and notices win.
    pub(crate) fn render(
        &mut self,
        available: Rect,
        buf: &mut Buffer,
        empty: bool,
        now: Instant,
    ) -> Option<Duration> {
        let stage = renderer::stage(available, buf);
        let Some(stage) = stage.filter(|_| self.needs_space()) else {
            self.hide();
            return None;
        };
        let dt = self
            .last_frame
            .replace(now)
            .map_or(Duration::ZERO, |last| now.saturating_duration_since(last));
        let visible = self.eligible && empty;
        let previous = self.opacity;
        let step = dt.as_secs_f32() / FADE_DURATION;
        self.opacity = (self.opacity + if visible { step } else { -step }).clamp(0.0, 1.0);
        if visible && previous == 1.0 {
            self.elapsed += dt;
        }
        if self.opacity > 0.0 {
            self.renderer.paint(stage, buf, self.elapsed, self.opacity);
        }
        if !visible && self.opacity == 0.0 {
            self.last_frame = None;
            return None;
        }
        Some(if self.opacity < 1.0 {
            Duration::from_millis(33)
        } else {
            FRAME_INTERVAL
        })
    }
}

#[cfg(test)]
mod tests;
