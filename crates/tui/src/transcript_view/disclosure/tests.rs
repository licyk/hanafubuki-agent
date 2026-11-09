// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Disclosing an activity's details from the keyboard and the pointer.

use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use super::*;
use crate::transcript_view::tests::fixtures::{cell, key, mouse, render, text, tool_call};

#[test]
fn f4_focuses_the_latest_activity_and_enter_toggles_its_details() {
    let cells = vec![
        tool_call("call-1", 12),
        cell("between"),
        tool_call("call-2", 30),
    ];
    let mut view = TranscriptView::default();
    let compact = text(&render(&mut view, &cells, 30, 40));
    assert!(compact.contains("+ 2 lines") && compact.contains("+ 20 lines"));
    assert_eq!(
        view.handle_key(key(KeyCode::F(4)), &cells),
        Some(ViewAction::Changed)
    );
    assert!(view.is_activity_focused());
    view.handle_key(key(KeyCode::Enter), &cells);
    let expanded = text(&render(&mut view, &cells, 30, 60));
    assert!(expanded.contains("out 15") && expanded.contains("− Show less"));
    // Up moves to the earlier activity; Right expands it, Left collapses.
    view.handle_key(key(KeyCode::Up), &cells);
    view.handle_key(key(KeyCode::Right), &cells);
    assert!(view.disclosure.is_expanded(&["tool:call-1".to_string()]));
    view.handle_key(key(KeyCode::Left), &cells);
    assert!(!view.disclosure.is_expanded(&["tool:call-1".to_string()]));
    // Typing leaves the focus to the composer.
    assert_eq!(view.handle_key(key(KeyCode::Char('x')), &cells), None);
    assert!(!view.is_activity_focused());
}

#[test]
fn a_click_on_the_control_toggles_and_the_detailed_view_has_none() {
    let cells = vec![tool_call("call-1", 30)];
    let mut view = TranscriptView::default();
    let frame = render(&mut view, &cells, 30, 14);
    let row = (0..14)
        .find(|&y| {
            (0..30)
                .map(|x| frame[(x, y)].symbol())
                .collect::<String>()
                .contains("+ 20 lines")
        })
        .expect("control row");
    let click = mouse(MouseEventKind::Down(MouseButton::Left), 8, row);
    assert_eq!(view.handle_mouse(click, &cells), Some(ViewAction::Changed));
    assert!(text(&render(&mut view, &cells, 30, 40)).contains("out 15"));
    view.set_presentation(true, HistoryRenderMode::Rich);
    let detailed = text(&render(&mut view, &cells, 30, 40));
    assert!(detailed.contains("out 15") && !detailed.contains("Show less"));
    assert_eq!(view.handle_key(key(KeyCode::F(4)), &cells), None);
}

#[test]
fn a_short_output_has_nothing_to_disclose() {
    let cells = vec![tool_call("call-1", 3)];
    let mut view = TranscriptView::default();
    let frame = text(&render(&mut view, &cells, 30, 8));
    assert!(!frame.contains("Show") && !frame.contains("lines"));
    view.handle_key(key(KeyCode::F(4)), &cells);
    assert!(!view.is_activity_focused());
}
