//! Fresh-conversation decoration, inspired by Codex's empty-state animation.
//! It is painted into the viewport, never inserted into conversation history.

mod renderer;

use hanafubuki_agent::{AgentTuiPreferences, AgentWelcomeTheme};
use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant};

use ratatui::{buffer::Buffer, layout::Rect};

use renderer::Renderer;

pub(crate) const FRAME_INTERVAL: Duration = Duration::from_millis(83);
const FADE_DURATION: f32 = 0.45;

pub(crate) struct Welcome {
    eligible: bool,
    enabled: bool,
    choice: AgentWelcomeTheme,
    random_theme: AgentWelcomeTheme,
    opacity: f32,
    elapsed: Duration,
    last_frame: Option<Instant>,
    renderer: Renderer,
}

impl Welcome {
    pub(crate) fn new(eligible: bool) -> Self {
        let themes = [
            AgentWelcomeTheme::Center,
            AgentWelcomeTheme::LeftPeek,
            AgentWelcomeTheme::RightSleep,
            AgentWelcomeTheme::RightRead,
            AgentWelcomeTheme::LowerLeft,
        ];
        let index = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish() as usize
            % themes.len();
        let random_theme = themes[index];
        Self {
            eligible,
            enabled: true,
            choice: AgentWelcomeTheme::Random,
            random_theme,
            opacity: 0.0,
            elapsed: Duration::ZERO,
            last_frame: None,
            renderer: Renderer::new(random_theme),
        }
    }

    pub(crate) fn configure(&mut self, preferences: &AgentTuiPreferences) {
        let choice = preferences.welcome_theme;
        if self.choice != choice {
            self.choice = choice;
            self.renderer = Renderer::new(if choice == AgentWelcomeTheme::Random {
                self.random_theme
            } else {
                choice
            });
            self.elapsed = Duration::ZERO;
            self.hide();
        }
        self.enabled = preferences.welcome_enabled;
        if !self.enabled {
            self.hide();
        }
    }

    pub(crate) fn dismiss(&mut self) {
        self.eligible = false;
    }

    pub(crate) fn needs_space(&self) -> bool {
        self.enabled && (self.eligible || self.opacity > 0.0)
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
        if !self.needs_space() {
            self.hide();
            return None;
        }
        let Some(stage) = self.renderer.stage(available, buf) else {
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
