// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The footer the view hands its caller, in each interaction.

use super::*;
use crate::transcript_view::tests::fixtures::{cell, key, render, tool_call};
use ratatui::crossterm::event::KeyCode;

fn footer_text(view: &TranscriptView, width: u16) -> Option<Vec<String>> {
    view.footer(width)
        .map(|footer| footer.lines.iter().map(ToString::to_string).collect())
}

#[test]
fn following_shows_no_footer_and_a_reader_is_offered_the_latest() {
    let mut cells = vec![cell("one\ntwo\nthree\nfour")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 40, 2);
    assert_eq!(footer_text(&view, 40), None);
    view.scroll(&cells, -1);
    render(&mut view, &cells, 40, 2);
    assert_eq!(footer_text(&view, 40), Some(vec!["esc latest".to_string()]));
    cells.push(cell("new"));
    render(&mut view, &cells, 40, 2);
    assert_eq!(
        footer_text(&view, 40),
        Some(vec!["New activity · esc latest".to_string()])
    );
    view.history = TranscriptHistoryState::LoadingOlder;
    assert_eq!(footer_text(&view, 12), Some(vec!["esc latest".to_string()]));
}

#[test]
fn the_query_is_shown_with_its_caret_and_the_search_status() {
    let cells = vec![cell("needle")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 40, 2);
    view.begin_search();
    view.paste_search("abcdefghijklmno");
    let footer = view.footer(14).expect("search footer");
    assert_eq!(footer.lines[0].to_string(), "Find: ijklmno ");
    assert_eq!(footer.cursor_column, Some(13));
    view.handle_key(key(KeyCode::Home), &cells);
    let footer = view.footer(80).expect("search footer");
    assert_eq!(footer.cursor_column, Some(6));
    assert_eq!(
        footer.lines[1].to_string(),
        "Searching… · full transcript · esc cancel"
    );
}

#[test]
fn a_selection_select_mode_and_a_focused_activity_have_their_hints() {
    let cells = vec![tool_call("call-1", 30)];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 40, 20);
    view.begin_selection(&cells, 0, 0, 2);
    assert_eq!(
        footer_text(&view, 80),
        Some(vec![
            "ctrl+c copy · enter copy & follow · esc clear".to_string()
        ])
    );
    assert_eq!(
        footer_text(&view, 20),
        Some(vec!["enter copy+↓ · esc".to_string()])
    );
    view.end_selection(&cells);
    view.handle_key(key(KeyCode::F(4)), &cells);
    assert_eq!(
        footer_text(&view, 80),
        Some(vec![
            "↑/↓ previous/next · enter details · esc back".to_string()
        ])
    );
    view.handle_key(key(KeyCode::Esc), &cells);
    assert!(view.begin_select_mode(&cells));
    assert_eq!(
        footer_text(&view, 30),
        Some(vec!["v mark · enter/q close".to_string()])
    );
}
