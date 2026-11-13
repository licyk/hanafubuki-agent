// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::time::{Duration, Instant};

use ratatui::text::Line;

use super::StreamState;

#[test]
fn drain_n_clamps_to_available_lines() {
    let mut state = StreamState::new();
    state.enqueue(vec![Line::from("one")], Instant::now());

    let drained = state.drain_n(8);
    assert_eq!(drained, vec![Line::from("one")]);
    assert!(state.is_idle());
}

#[test]
fn the_oldest_queued_row_gives_the_queue_its_age() {
    let mut state = StreamState::new();
    let t0 = Instant::now();
    state.enqueue(vec![Line::from("one"), Line::from("two")], t0);
    state.enqueue(vec![Line::from("three")], t0 + Duration::from_millis(30));
    assert_eq!(
        state.oldest_queued_age(t0 + Duration::from_millis(50)),
        Some(Duration::from_millis(50))
    );
    state.step();
    state.step();
    assert_eq!(
        state.oldest_queued_age(t0 + Duration::from_millis(50)),
        Some(Duration::from_millis(20))
    );
    state.step();
    assert_eq!(state.oldest_queued_age(t0), None);
}
