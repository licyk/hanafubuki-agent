// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;
use crate::app_event::AppEvent;
use crate::app_event::AppEventSender;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::sync::mpsc::channel as unbounded_channel;
use std::time::Duration;
use std::time::Instant;

#[test]
fn fmt_elapsed_compact_formats_seconds_minutes_hours() {
    assert_eq!(fmt_elapsed_compact(0), "0s");
    assert_eq!(fmt_elapsed_compact(1), "1s");
    assert_eq!(fmt_elapsed_compact(59), "59s");
    assert_eq!(fmt_elapsed_compact(60), "1m 00s");
    assert_eq!(fmt_elapsed_compact(61), "1m 01s");
    assert_eq!(fmt_elapsed_compact(3 * 60 + 5), "3m 05s");
    assert_eq!(fmt_elapsed_compact(59 * 60 + 59), "59m 59s");
    assert_eq!(fmt_elapsed_compact(3600), "1h 00m 00s");
    assert_eq!(fmt_elapsed_compact(3600 + 60 + 1), "1h 01m 01s");
    assert_eq!(fmt_elapsed_compact(25 * 3600 + 2 * 60 + 3), "25h 02m 03s");
}

#[test]
fn renders_with_working_header() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let w = StatusIndicatorWidget::new(tx, crate::tui::FrameRequester::test_dummy(), true);

    // Render into a fixed-size test terminal and snapshot the backend.
    let mut terminal = Terminal::new(TestBackend::new(80, 2)).expect("terminal");
    terminal
        .draw(|f| w.render(f.area(), f.buffer_mut()))
        .expect("draw");
    insta::assert_snapshot!(terminal.backend());
}

#[test]
fn renders_truncated() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let w = StatusIndicatorWidget::new(tx, crate::tui::FrameRequester::test_dummy(), true);

    // Render into a fixed-size test terminal and snapshot the backend.
    let mut terminal = Terminal::new(TestBackend::new(20, 2)).expect("terminal");
    terminal
        .draw(|f| w.render(f.area(), f.buffer_mut()))
        .expect("draw");
    insta::assert_snapshot!(terminal.backend());
}

#[test]
fn renders_wrapped_details_panama_two_lines() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut w = StatusIndicatorWidget::new(tx, crate::tui::FrameRequester::test_dummy(), false);
    w.update_details(Some("A man a plan a canal panama".to_string()));
    w.set_interrupt_hint_visible(false);

    // Freeze time-dependent rendering (elapsed + spinner) to keep the snapshot stable.
    w.is_paused = true;
    w.elapsed_running = Duration::ZERO;

    // Prefix is 4 columns, so a width of 30 yields a content width of 26: one column
    // short of fitting the whole phrase (27 cols), forcing exactly one wrap without ellipsis.
    let mut terminal = Terminal::new(TestBackend::new(30, 3)).expect("terminal");
    terminal
        .draw(|f| w.render(f.area(), f.buffer_mut()))
        .expect("draw");
    insta::assert_snapshot!(terminal.backend());
}

#[test]
fn timer_pauses_when_requested() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut widget = StatusIndicatorWidget::new(tx, crate::tui::FrameRequester::test_dummy(), true);

    let baseline = Instant::now();
    widget.last_resume_at = baseline;

    let before_pause = widget.elapsed_seconds_at(baseline + Duration::from_secs(5));
    assert_eq!(before_pause, 5);

    widget.pause_timer_at(baseline + Duration::from_secs(5));
    let paused_elapsed = widget.elapsed_seconds_at(baseline + Duration::from_secs(10));
    assert_eq!(paused_elapsed, before_pause);

    widget.resume_timer_at(baseline + Duration::from_secs(10));
    let after_resume = widget.elapsed_seconds_at(baseline + Duration::from_secs(13));
    assert_eq!(after_resume, before_pause + 3);
}

#[test]
fn details_overflow_adds_ellipsis() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut w = StatusIndicatorWidget::new(tx, crate::tui::FrameRequester::test_dummy(), true);
    w.update_details(Some("abcd abcd abcd abcd".to_string()));

    let lines = w.wrapped_details_lines(6);
    assert_eq!(lines.len(), DETAILS_MAX_LINES);
    let last = lines.last().expect("expected last details line");
    assert!(
        last.spans[1].content.as_ref().ends_with("…"),
        "expected ellipsis in last line: {last:?}"
    );
}

#[test]
fn fmt_elapsed_tenths_cuts_to_tenths_under_a_minute() {
    let tenths = |millis| fmt_elapsed_tenths(Duration::from_millis(millis));
    assert_eq!(tenths(0), "0.0s");
    assert_eq!(tenths(3_449), "3.4s");
    assert_eq!(tenths(20_790), "20.7s");
    assert_eq!(tenths(59_960), "59.9s");
    assert_eq!(tenths(65_000), "1m 05s");
}

#[test]
fn the_thinking_clock_counts_what_completed_and_the_open_stretch() {
    let now = Instant::now();
    let epoch = 1_000_000;
    let snapshot = |turn_completed_millis, active_since| AgentReasoningSnapshot {
        sampling_step: 2,
        turn_completed_millis,
        active_since,
        ..AgentReasoningSnapshot::default()
    };
    // Not thought yet, including a step's reset of a turn that has not.
    assert_eq!(
        ThinkingClock::from_snapshot(&snapshot(0, None), now, epoch).total_at(now),
        None
    );
    // Closed stretches: the clock stands.
    let closed = ThinkingClock::from_snapshot(&snapshot(20_790, None), now, epoch);
    assert_eq!(
        closed.total_at(now + Duration::from_secs(9)),
        Some(Duration::from_millis(20_790))
    );
    // An open one, begun 3 s before the snapshot was read, runs on.
    let open = ThinkingClock::from_snapshot(&snapshot(20_790, Some(epoch - 3_000)), now, epoch);
    assert_eq!(
        open.total_at(now + Duration::from_millis(500)),
        Some(Duration::from_millis(24_290))
    );
}

#[test]
fn the_status_row_shows_the_turns_thinking_and_pauses_only_working() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut w = StatusIndicatorWidget::new(tx, crate::tui::FrameRequester::test_dummy(), false);
    // An approval waits: the turn's timer stops at 25 s, and the thinking
    // that closed before it stands.
    w.is_paused = true;
    w.elapsed_running = Duration::from_secs(25);
    w.thinking = ThinkingClock {
        completed: Duration::from_millis(20_790),
        active_since: None,
    };
    let mut terminal = Terminal::new(TestBackend::new(60, 1)).expect("terminal");
    terminal
        .draw(|f| w.render(f.area(), f.buffer_mut()))
        .expect("draw");
    let row: String = (0..60)
        .map(|x| terminal.backend().buffer()[(x, 0)].symbol().to_string())
        .collect();
    assert_eq!(
        row.trim_end(),
        "• Working 25s (Thinking 20.7s) • esc to interrupt"
    );
    assert_eq!(w.elapsed_seconds(), 25);
    assert_eq!(w.thinking_duration(), Some(Duration::from_millis(20_790)));
}
