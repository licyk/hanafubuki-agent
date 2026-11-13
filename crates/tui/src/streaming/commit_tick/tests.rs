// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::time::{Duration, Instant};

use super::*;
use crate::markdown_render::ListSpacing;

fn answer(rows: usize, at: Instant) -> StreamController {
    let mut controller = StreamController::new(Some(80), String::new(), ListSpacing::default());
    let source: String = (0..rows).map(|row| format!("row {row}\n")).collect();
    controller.push_at(&source, at);
    controller
}

fn rows(cell: &Option<Box<dyn HistoryCell>>) -> usize {
    cell.as_ref()
        .map_or(0, |cell| cell.display_lines(u16::MAX).len())
}

#[test]
fn a_smooth_tick_commits_one_row() {
    let t0 = Instant::now();
    let mut policy = AdaptiveChunkingPolicy::default();
    let mut controller = answer(3, t0);
    for (tick, left) in [(1, 2), (2, 1), (3, 0)] {
        let output = run_commit_tick(
            &mut policy,
            Some(&mut controller),
            None,
            CommitTickScope::AnyMode,
            t0 + Duration::from_millis(8 * tick),
        );
        assert_eq!(rows(&output.answer), 1);
        assert_eq!(controller.queued_lines(), left);
        assert_eq!(output.all_idle, left == 0);
        assert!(output.has_controller);
    }
}

#[test]
fn a_deep_queue_is_caught_up_at_once() {
    let t0 = Instant::now();
    let mut policy = AdaptiveChunkingPolicy::default();
    let mut controller = answer(12, t0);
    let output = run_commit_tick(
        &mut policy,
        Some(&mut controller),
        None,
        CommitTickScope::AnyMode,
        t0 + Duration::from_millis(8),
    );
    assert_eq!(rows(&output.answer), 12);
    assert!(output.all_idle);
}

#[test]
fn an_old_queue_is_caught_up_at_once() {
    let t0 = Instant::now();
    let mut policy = AdaptiveChunkingPolicy::default();
    let mut controller = answer(3, t0);
    let output = run_commit_tick(
        &mut policy,
        Some(&mut controller),
        None,
        CommitTickScope::AnyMode,
        t0 + Duration::from_millis(120),
    );
    assert_eq!(rows(&output.answer), 3);
}

#[test]
fn a_delta_commits_only_while_catching_up() {
    let t0 = Instant::now();
    let mut policy = AdaptiveChunkingPolicy::default();
    let mut controller = answer(3, t0);
    let output = run_commit_tick(
        &mut policy,
        Some(&mut controller),
        None,
        CommitTickScope::CatchUpOnly,
        t0,
    );
    assert_eq!(rows(&output.answer), 0);
    assert_eq!(controller.queued_lines(), 3);

    controller.push_at(&"more\n".repeat(8), t0);
    let output = run_commit_tick(
        &mut policy,
        Some(&mut controller),
        None,
        CommitTickScope::CatchUpOnly,
        t0,
    );
    assert!(rows(&output.answer) > 8);
    assert_eq!(controller.queued_lines(), 0);
}

#[test]
fn both_streams_count_toward_the_pressure() {
    let t0 = Instant::now();
    let mut policy = AdaptiveChunkingPolicy::default();
    let mut controller = answer(4, t0);
    let mut plan = PlanStreamController::new(Some(80), String::new(), ListSpacing::default());
    plan.push_at(&"step\n\n".repeat(4), t0);
    let output = run_commit_tick(
        &mut policy,
        Some(&mut controller),
        Some(&mut plan),
        CommitTickScope::AnyMode,
        t0 + Duration::from_millis(8),
    );
    assert_eq!(rows(&output.answer), 4);
    assert!(output.plan.is_some());
    assert!(output.all_idle);
    assert_eq!(plan.queued_lines(), 0);
}
