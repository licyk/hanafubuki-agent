// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! A live status indicator that shows the *latest* log line emitted by the
//! application while the agent is processing a long‑running task, and the
//! line that takes its place while the session waits on its background
//! work ([`waiting_line`]).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hanafubuki_agent::{AgentBackgroundStatus, AgentReasoningSnapshot, AgentWakeState};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use crate::app_event::{AppEvent, AppEventSender, Op};
use crate::keymap::{self, Action};
use crate::render::renderable::Renderable;
use crate::shimmer::{shimmer_spans, spinner};
use crate::text_formatting::capitalize_first;
use crate::tui::FrameRequester;
use crate::wrapping::word_wrap_lines;
use crate::wrapping::RtOptions;

const DETAILS_MAX_LINES: usize = 3;

/// What shows in place of the status indicator while the session runs no
/// turn and its background work does: that it resumes by itself, or that
/// the person's message resumes it. `None` when nothing holds it open, or
/// it is not woken for it.
pub(crate) fn waiting_line(background: &AgentBackgroundStatus) -> Option<String> {
    let count = |count: usize, one: &str| match count {
        0 => None,
        1 => Some(format!("1 {one}")),
        count => Some(format!("{count} {one}s")),
    };
    let work = [
        count(background.work.agents, "agent"),
        count(background.work.terminals, "terminal"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    if work.is_empty() {
        return None;
    }
    match background.wake {
        AgentWakeState::On => Some(format!("Waiting for {work} — resumes by itself · /tasks")),
        AgentWakeState::Paused { .. } => Some(format!(
            "Paused: {work} still running — send a message to resume"
        )),
        AgentWakeState::Off => None,
    }
}
const DETAILS_PREFIX: &str = "  └ ";

pub(crate) struct StatusIndicatorWidget {
    /// Animated header text (defaults to "Working").
    header: String,
    details: Option<String>,
    show_interrupt_hint: bool,

    elapsed_running: Duration,
    last_resume_at: Instant,
    is_paused: bool,
    thinking: ThinkingClock,
    app_event_tx: AppEventSender,
    frame_requester: FrameRequester,
    animations_enabled: bool,
}

// Format elapsed seconds into a compact human-friendly form used by the status line.
// Examples: 0s, 59s, 1m 00s, 59m 59s, 1h 00m 00s, 2h 03m 09s
pub(crate) fn fmt_elapsed_compact(elapsed_secs: u64) -> String {
    if elapsed_secs < 60 {
        return format!("{elapsed_secs}s");
    }
    if elapsed_secs < 3600 {
        let minutes = elapsed_secs / 60;
        let seconds = elapsed_secs % 60;
        return format!("{minutes}m {seconds:02}s");
    }
    let hours = elapsed_secs / 3600;
    let minutes = (elapsed_secs % 3600) / 60;
    let seconds = elapsed_secs % 60;
    format!("{hours}h {minutes:02}m {seconds:02}s")
}

/// A thinking's time: in tenths of a second, cut rather than rounded,
/// under a minute (`20.7s`), as [`fmt_elapsed_compact`] above.
pub(crate) fn fmt_elapsed_tenths(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    if millis < 60_000 {
        return format!("{}.{}s", millis / 1000, millis % 1000 / 100);
    }
    fmt_elapsed_compact(elapsed.as_secs())
}

/// The turn's thinking time: what its finished stretches took, and since
/// when it thinks now.
#[derive(Clone, Copy, Debug, Default)]
struct ThinkingClock {
    completed: Duration,
    active_since: Option<Instant>,
}

impl ThinkingClock {
    /// The kernel's snapshot, whose times are milliseconds since the Unix
    /// epoch, read against `now` and the epoch's `epoch_millis`.
    fn from_snapshot(reasoning: &AgentReasoningSnapshot, now: Instant, epoch_millis: u128) -> Self {
        let millis =
            |millis: u128| Duration::from_millis(u64::try_from(millis).unwrap_or(u64::MAX));
        Self {
            completed: millis(reasoning.turn_completed_millis),
            active_since: reasoning.active_since.map(|since| {
                now.checked_sub(millis(epoch_millis.saturating_sub(since)))
                    .unwrap_or(now)
            }),
        }
    }

    /// `None` until the turn has thought.
    fn total_at(&self, now: Instant) -> Option<Duration> {
        let active = self
            .active_since
            .map(|since| now.saturating_duration_since(since));
        (!self.completed.is_zero() || active.is_some())
            .then(|| self.completed + active.unwrap_or_default())
    }
}

impl StatusIndicatorWidget {
    pub(crate) fn new(
        app_event_tx: AppEventSender,
        frame_requester: FrameRequester,
        animations_enabled: bool,
    ) -> Self {
        Self {
            header: String::from("Working"),
            details: None,
            show_interrupt_hint: true,
            elapsed_running: Duration::ZERO,
            last_resume_at: Instant::now(),
            is_paused: false,
            thinking: ThinkingClock::default(),

            app_event_tx,
            frame_requester,
            animations_enabled,
        }
    }

    pub(crate) fn interrupt(&self) {
        self.app_event_tx.send(AppEvent::Op(Op::Interrupt));
    }

    /// Update the animated header label (left of the brackets).
    pub(crate) fn update_header(&mut self, header: String) {
        self.header = header;
    }

    /// Update the details text shown below the header.
    pub(crate) fn update_details(&mut self, details: Option<String>) {
        self.details = details
            .filter(|details| !details.is_empty())
            .map(|details| capitalize_first(details.trim_start()));
    }

    pub(crate) fn set_interrupt_hint_visible(&mut self, visible: bool) {
        self.show_interrupt_hint = visible;
    }

    /// The turn's thinking as the kernel last reported it.
    pub(crate) fn update_reasoning(&mut self, reasoning: &AgentReasoningSnapshot) {
        let epoch_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        self.thinking = ThinkingClock::from_snapshot(reasoning, Instant::now(), epoch_millis);
    }

    /// How long the turn has thought, once it has.
    pub(crate) fn thinking_duration(&self) -> Option<Duration> {
        self.thinking.total_at(Instant::now())
    }

    pub(crate) fn pause_timer(&mut self) {
        self.pause_timer_at(Instant::now());
    }

    pub(crate) fn resume_timer(&mut self) {
        self.resume_timer_at(Instant::now());
    }

    pub(crate) fn pause_timer_at(&mut self, now: Instant) {
        if self.is_paused {
            return;
        }
        self.elapsed_running += now.saturating_duration_since(self.last_resume_at);
        self.is_paused = true;
    }

    pub(crate) fn resume_timer_at(&mut self, now: Instant) {
        if !self.is_paused {
            return;
        }
        self.last_resume_at = now;
        self.is_paused = false;
        self.frame_requester.schedule_frame();
    }

    fn elapsed_duration_at(&self, now: Instant) -> Duration {
        let mut elapsed = self.elapsed_running;
        if !self.is_paused {
            elapsed += now.saturating_duration_since(self.last_resume_at);
        }
        elapsed
    }

    fn elapsed_seconds_at(&self, now: Instant) -> u64 {
        self.elapsed_duration_at(now).as_secs()
    }

    pub(crate) fn elapsed_seconds(&self) -> u64 {
        self.elapsed_seconds_at(Instant::now())
    }

    /// Wrap the details text into a fixed width and return the lines, truncating if necessary.
    fn wrapped_details_lines(&self, width: u16) -> Vec<Line<'static>> {
        let Some(details) = self.details.as_deref() else {
            return Vec::new();
        };
        if width == 0 {
            return Vec::new();
        }

        let prefix_width = UnicodeWidthStr::width(DETAILS_PREFIX);
        let opts = RtOptions::new(usize::from(width))
            .initial_indent(Line::from(DETAILS_PREFIX.dim()))
            .subsequent_indent(Line::from(Span::from(" ".repeat(prefix_width)).dim()))
            .break_words(true);

        let mut out = word_wrap_lines(details.lines().map(|line| vec![line.dim()]), opts);

        if out.len() > DETAILS_MAX_LINES {
            out.truncate(DETAILS_MAX_LINES);
            let content_width = usize::from(width).saturating_sub(prefix_width).max(1);
            let max_base_len = content_width.saturating_sub(1);
            if let Some(span) = out.last_mut().and_then(|last| last.spans.last_mut()) {
                let trimmed: String = span.content.as_ref().chars().take(max_base_len).collect();
                *span = format!("{trimmed}…").dim();
            }
        }

        out
    }
}

impl Renderable for StatusIndicatorWidget {
    fn desired_height(&self, width: u16) -> u16 {
        1 + u16::try_from(self.wrapped_details_lines(width).len()).unwrap_or(0)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }

        // Schedule next animation frame.
        self.frame_requester
            .schedule_frame_in(Duration::from_millis(32));
        let now = Instant::now();
        let elapsed_duration = self.elapsed_duration_at(now);
        let pretty_elapsed = fmt_elapsed_compact(elapsed_duration.as_secs());

        let mut spans = Vec::with_capacity(8);
        spans.push(spinner(Some(self.last_resume_at), self.animations_enabled));
        spans.push(" ".into());
        if self.animations_enabled {
            spans.extend(shimmer_spans(&self.header));
        } else if !self.header.is_empty() {
            spans.push(self.header.clone().into());
        }
        spans.push(format!(" {pretty_elapsed}").dim());
        if let Some(thinking) = self.thinking.total_at(now) {
            spans.push(format!(" (Thinking {})", fmt_elapsed_tenths(thinking)).dim());
        }
        if let Some(key) = keymap::hint(Action::InterruptTurn).filter(|_| self.show_interrupt_hint)
        {
            spans.extend(vec![" • ".dim(), key.into(), " to interrupt".dim()]);
        }

        let mut lines = Vec::new();
        lines.push(Line::from(spans));
        if area.height > 1 {
            // If there is enough space, add the details lines below the header.
            let details = self.wrapped_details_lines(area.width);
            let max_details = usize::from(area.height.saturating_sub(1));
            lines.extend(details.into_iter().take(max_details));
        }

        Paragraph::new(Text::from(lines)).render(area, buf);
    }
}

#[cfg(test)]
mod tests;
