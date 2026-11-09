// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Mouse and keyboard selection over retained revisions.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::style::Modifier;

use super::*;
use crate::transcript_view::tests::fixtures::{cell, ctrl, key, mouse, render};

#[test]
fn selection_marks_hard_breaks_within_and_between_entries() {
    for (cells, end_row) in [
        (vec![cell("code\nnext")], 1),
        (vec![cell("code"), cell("next")], 2),
    ] {
        let mut view = TranscriptView::default();
        render(&mut view, &cells, 8, 3);
        view.begin_selection(&cells, 0, 0, 1);
        view.extend_selection(7, 0);
        let buffer = render(&mut view, &cells, 8, 3);
        assert_eq!(view.selected_text(&cells).as_deref(), Some("code"));
        assert!(!buffer[(4, 0)].modifier.contains(Modifier::REVERSED));
        view.extend_selection(0, end_row);
        let buffer = render(&mut view, &cells, 8, 3);
        assert_eq!(view.selected_text(&cells).as_deref(), Some("code\n"));
        assert!(buffer[(4, 0)].modifier.contains(Modifier::REVERSED));
    }
}

#[test]
fn copy_keys_hand_back_the_selection_and_enter_also_follows() {
    let cells = vec![cell("alpha beta")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 3);
    view.begin_selection(&cells, 6, 0, 2);
    assert_eq!(
        view.handle_key(ctrl('c'), &cells),
        Some(ViewAction::Copy("beta".to_string()))
    );
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
        KeyEventKind::Release,
    );
    assert_eq!(view.handle_key(release, &cells), None);
    assert_eq!(
        view.handle_key(key(KeyCode::Enter), &cells),
        Some(ViewAction::CopyAndFollow("beta".to_string()))
    );
    // With nothing selected, Enter copies nothing and stays in the view.
    view.end_selection(&cells);
    view.begin_selection(&cells, 6, 0, 1);
    assert_eq!(
        view.handle_key(key(KeyCode::Enter), &cells),
        Some(ViewAction::Changed)
    );
}

#[test]
fn a_right_click_copies_the_selection() {
    let cells = vec![cell("alpha beta")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 3);
    let right = mouse(MouseEventKind::Down(MouseButton::Right), 0, 0);
    assert_eq!(view.handle_mouse(right, &cells), None);
    view.begin_selection(&cells, 0, 0, 2);
    assert_eq!(
        view.handle_mouse(right, &cells),
        Some(ViewAction::Copy("alpha".to_string()))
    );
}

#[test]
fn word_and_line_drags_keep_their_unit_when_the_direction_changes() {
    let cells = vec![cell("alpha beta gamma\nsecond line\nthird")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 32, 3);
    view.begin_selection(&cells, 9, 0, 2);
    view.extend_selection(14, 0);
    assert_eq!(view.selected_text(&cells).as_deref(), Some("beta gamma"));
    view.extend_selection(3, 0);
    assert_eq!(view.selected_text(&cells).as_deref(), Some("alpha beta"));
    view.extend_selection(14, 0);
    assert_eq!(view.selected_text(&cells).as_deref(), Some("beta gamma"));

    view.begin_selection(&cells, 4, 1, 3);
    view.extend_selection(3, 0);
    assert_eq!(
        view.selected_text(&cells).as_deref(),
        Some("alpha beta gamma\nsecond line\n")
    );
    view.extend_selection(4, 2);
    assert_eq!(
        view.selected_text(&cells).as_deref(),
        Some("second line\nthird")
    );
}

#[test]
fn keyboard_selection_moves_by_grapheme_and_keeps_its_column_across_short_lines() {
    let cells = vec![cell("A👨‍👩‍👧‍👦界e\u{301}Z\nx\n1234567890"), cell("tail")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 32, 6);
    view.begin_selection(&cells, 0, 0, 1);
    for _ in 0..4 {
        view.selection_key(&cells, KeyCode::Right);
    }
    assert_eq!(view.selected_text(&cells).as_deref(), Some("A👨‍👩‍👧‍👦界e\u{301}"));
    let original = view.selection.as_ref().expect("selection").end;
    for code in [KeyCode::Down, KeyCode::Down, KeyCode::Up, KeyCode::Up] {
        view.selection_key(&cells, code);
    }
    assert_eq!(view.selection.as_ref().expect("selection").end, original);
    for _ in 0..10 {
        view.selection_key(&cells, KeyCode::Down);
    }
    let end = view.selection.as_ref().expect("selection").end;
    assert!(end.index < cells.len());
    let layout = view.layout(&cells, end.index).expect("endpoint layout");
    assert!(layout.text().is_char_boundary(end.offset));
}

#[test]
fn plain_and_shift_arrows_move_the_selection_on_press_and_repeat() {
    let cells = vec![cell("abc\ndef\nghi")];
    for kind in [KeyEventKind::Press, KeyEventKind::Repeat] {
        for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
            for (code, expected) in [
                (KeyCode::Left, "d"),
                (KeyCode::Right, "e"),
                (KeyCode::Up, "bc\nd"),
                (KeyCode::Down, "ef\ng"),
            ] {
                let mut view = TranscriptView::default();
                render(&mut view, &cells, 32, 3);
                view.begin_selection(&cells, 1, 1, 1);
                assert_eq!(
                    view.handle_key(KeyEvent::new_with_kind(code, modifiers, kind), &cells),
                    Some(ViewAction::Changed)
                );
                assert_eq!(view.selected_text(&cells).as_deref(), Some(expected));
            }
        }
    }
}

#[test]
fn modified_arrows_end_the_selection_and_return_input_to_the_caller() {
    let cells = vec![cell("alpha beta gamma")];
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ] {
        for code in [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down] {
            let mut view = TranscriptView::default();
            render(&mut view, &cells, 32, 3);
            view.begin_selection(&cells, 9, 0, 2);
            assert_eq!(view.selected_text(&cells).as_deref(), Some("beta"));
            assert_eq!(
                view.handle_key(KeyEvent::new(code, modifiers), &cells),
                None
            );
            assert_eq!(
                (view.has_active_interaction(), view.selected_text(&cells)),
                (false, None)
            );
        }
    }
}

#[test]
fn shift_click_extends_a_double_clicked_word_both_ways() {
    let cells = vec![cell("alpha beta gamma")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 24, 3);
    for _ in 0..2 {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            view.handle_mouse(mouse(kind, 7, 0), &cells);
            render(&mut view, &cells, 24, 3);
        }
    }
    assert_eq!(view.selected_text(&cells).as_deref(), Some("beta"));
    for (column, expected) in [(13, "beta gamma"), (1, "alpha beta"), (13, "beta gamma")] {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            let shifted = MouseEvent {
                modifiers: KeyModifiers::SHIFT,
                ..mouse(kind, column, 0)
            };
            assert_eq!(
                view.handle_mouse(shifted, &cells),
                Some(ViewAction::Changed)
            );
            render(&mut view, &cells, 24, 3);
            assert_eq!(view.selected_text(&cells).as_deref(), Some(expected));
        }
    }
    // A plain click afterwards starts a fresh, empty selection.
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        view.handle_mouse(mouse(kind, 13, 0), &cells);
    }
    assert_eq!(view.selected_text(&cells), None);
}

#[test]
fn a_selection_keeps_its_revision_across_prepended_history_and_new_commits() {
    let header = cell("header");
    let selected = cell("selected");
    let mut cells = vec![Arc::clone(&header), Arc::clone(&selected)];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 32, 5);
    view.begin_selection(&cells, 4, 2, 2);
    view.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 4, 2), &cells);
    cells.push(cell("new commit"));
    let older = cell("older");
    cells.insert(1, Arc::clone(&older));
    view.history_loaded(&cells, 1..2);
    view.history_loaded(&cells, 1..2);
    assert_eq!(
        view.snapshot_cells()
            .expect("snapshot")
            .iter()
            .map(EntryKey::cell)
            .collect::<Vec<_>>(),
        vec![
            EntryKey::cell(&header),
            EntryKey::cell(&older),
            EntryKey::cell(&selected)
        ],
    );
    render(&mut view, &cells, 32, 6);
    assert_eq!(view.selected_text(&cells).as_deref(), Some("selected"));
}

#[test]
fn dragging_past_the_edge_scrolls_until_the_pointer_comes_back() {
    let cells = vec![cell(
        (0..40)
            .map(|row| format!("row {row:02}\n"))
            .collect::<String>(),
    )];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 6);
    view.scroll(&cells, -12);
    render(&mut view, &cells, 20, 6);
    view.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0, 2), &cells);
    // A drag along the starting row does not scroll.
    view.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 3, 2), &cells);
    assert!(!view.tick_selection(&cells));
    view.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 3, 0), &cells);
    render(&mut view, &cells, 20, 6);
    assert!(view.tick_selection(&cells));
    // The wheel supersedes the pointer until the next drag.
    view.handle_mouse(mouse(MouseEventKind::ScrollUp, 3, 0), &cells);
    assert!(!view.tick_selection(&cells));
    view.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 3, 3), &cells);
    render(&mut view, &cells, 20, 6);
    assert!(!view.tick_selection(&cells));
    view.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 3, 3), &cells);
    // The pointer held the fourth row after the wheel moved the view.
    assert_eq!(
        view.selected_text(&cells).as_deref(),
        Some(" 21\nrow 22\nrow 23\n")
    );
}
