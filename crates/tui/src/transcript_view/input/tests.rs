// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Which keys the view takes, and the jumps to the transcript's ends.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::transcript_view::tests::fixtures::{cell, ctrl, key, render, text};

#[test]
fn the_jumps_accept_the_alt_alternatives_terminals_send() {
    for (code, modifiers, target) in [
        (KeyCode::Home, KeyModifiers::CONTROL, JumpTarget::Beginning),
        (KeyCode::End, KeyModifiers::CONTROL, JumpTarget::Latest),
        (KeyCode::Char('<'), KeyModifiers::ALT, JumpTarget::Beginning),
        (
            KeyCode::Char('<'),
            KeyModifiers::ALT | KeyModifiers::SHIFT,
            JumpTarget::Beginning,
        ),
        (
            KeyCode::Char(','),
            KeyModifiers::ALT | KeyModifiers::SHIFT,
            JumpTarget::Beginning,
        ),
        (KeyCode::Char('>'), KeyModifiers::ALT, JumpTarget::Latest),
        (
            KeyCode::Char('.'),
            KeyModifiers::ALT | KeyModifiers::SHIFT,
            JumpTarget::Latest,
        ),
    ] {
        assert!(JumpTarget::from_key(KeyEvent::new(code, modifiers)) == Some(target));
    }
    assert!(JumpTarget::from_key(key(KeyCode::Home)).is_none());
}

#[test]
fn the_jumps_and_esc_move_between_the_ends() {
    let cells = (0..10)
        .map(|index| cell(format!("entry {index}")))
        .collect::<Vec<_>>();
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 3);
    view.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL), &cells);
    assert_eq!(
        text(&render(&mut view, &cells, 20, 3)),
        "entry 0\n\nentry 1"
    );
    assert!(view.owns_interaction_key(key(KeyCode::Esc)) || view.can_return_to_latest());
    assert_eq!(
        view.handle_key(key(KeyCode::Esc), &cells),
        Some(ViewAction::Changed)
    );
    assert!(view.is_following());
    // At the latest rows, Esc is the caller's.
    assert_eq!(view.handle_key(key(KeyCode::Esc), &cells), None);
}

#[test]
fn the_view_takes_its_interaction_keys_first_and_leaves_typing() {
    let cells = vec![cell("alpha beta")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 3);
    assert!(view.owns_interaction_key(ctrl(' ')));
    assert!(!view.owns_interaction_key(key(KeyCode::Enter)));
    assert!(!view.owns_interaction_key(key(KeyCode::Char('a'))));
    view.begin_selection(&cells, 6, 0, 2);
    for owned in [
        ctrl('c'),
        key(KeyCode::Enter),
        key(KeyCode::Esc),
        key(KeyCode::Left),
    ] {
        assert!(view.owns_interaction_key(owned));
    }
    assert!(!view.owns_interaction_key(key(KeyCode::Char('a'))));
    view.end_selection(&cells);
    view.begin_search();
    assert!(view.owns_interaction_key(key(KeyCode::Enter)));
    assert!(view.owns_interaction_key(ctrl('n')));
}
