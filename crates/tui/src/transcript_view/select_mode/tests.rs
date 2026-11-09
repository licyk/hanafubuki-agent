// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The caret's motions, marks, and search inside select mode.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::*;
use crate::history_cell::PlainHistoryCell;
use crate::transcript_view::tests::fixtures::key;

fn cell(lines: &[&str]) -> Arc<dyn HistoryCell> {
    Arc::new(PlainHistoryCell::new(
        lines
            .iter()
            .map(|line| Line::from((*line).to_owned()))
            .collect(),
    ))
}

fn render(view: &mut TranscriptView, cells: &[Arc<dyn HistoryCell>]) -> Buffer {
    let area = Rect::new(0, 0, 32, 6);
    let mut buffer = Buffer::empty(area);
    view.render(area, &mut buffer, cells);
    buffer
}

fn press(view: &mut TranscriptView, cells: &[Arc<dyn HistoryCell>], code: KeyCode) {
    assert!(matches!(
        view.handle_key(KeyEvent::new(code, KeyModifiers::NONE), cells),
        Some(ViewAction::Changed)
    ));
}

#[test]
fn entry_starts_on_the_last_selectable_visible_line() {
    for (cells, expected) in [
        (vec![cell(&["top"]), cell(&["bottom"])], "b"),
        (vec![cell(&["text"]), cell(&[""])], "t"),
        (vec![cell(&["abc", "", "", "", "", "", ""])], "c"),
    ] {
        let mut view = TranscriptView::default();
        render(&mut view, &cells);
        assert!(view.begin_select_mode(&cells));
        press(&mut view, &cells, KeyCode::Char('v'));
        assert_eq!(view.selected_text(&cells).as_deref(), Some(expected));
    }

    let cells = vec![cell(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    view.scroll(&cells, -2);
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('v'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("h"));

    let cells = vec![cell(&["a", "", "", "", "", "", "", "", "", ""])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    assert!(matches!(view.position, Position::Reading(_)));

    let cells = vec![cell(&[""])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(!view.begin_select_mode(&cells));
}

#[test]
fn character_motion_crosses_line_and_entry_boundaries_without_invisible_stops() {
    let cells = vec![cell(&["a", ""]), cell(&[""]), cell(&["", "b"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Left);
    press(&mut view, &cells, KeyCode::Char('v'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("a"));

    press(&mut view, &cells, KeyCode::Esc);
    press(&mut view, &cells, KeyCode::Right);
    press(&mut view, &cells, KeyCode::Char('v'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("b"));

    let cells = vec![cell(&["ab"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    let separator = Anchor {
        key: EntryKey::cell(&cells[0]),
        index: 0,
        offset: 0,
        row_bias: 1,
    };
    view.selection
        .as_mut()
        .expect("selection")
        .set_keyboard_range(separator, separator);
    press(&mut view, &cells, KeyCode::Right);
    press(&mut view, &cells, KeyCode::Char('v'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("a"));

    let wrapped = format!("{} one", "1234567890".repeat(3));
    let cells = vec![cell(&[&wrapped])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Left);
    press(&mut view, &cells, KeyCode::Char('v'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("0"));

    let cells = vec![cell(&["ab"]), cell(&["cd"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    let start = Anchor {
        key: EntryKey::cell(&cells[0]),
        index: 0,
        offset: 0,
        row_bias: 0,
    };
    let end = Anchor {
        key: EntryKey::cell(&cells[1]),
        index: 1,
        offset: 0,
        row_bias: 0,
    };
    let mut selection = Selection::keyboard(view.capture_snapshot(&cells), start);
    selection.set_keyboard_range(start, end);
    view.selection = Some(selection);
    assert!(view.begin_select_mode(&cells));
    assert_eq!(
        view.select_mode
            .and_then(|mode| mode.mark)
            .expect("marked range")
            .active,
        Anchor { offset: 1, ..start }
    );
    assert_eq!(view.selected_text(&cells).as_deref(), Some("ab\n"));
    press(&mut view, &cells, KeyCode::Char('o'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("ab\n"));
    press(&mut view, &cells, KeyCode::Char('l'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("b"));

    let cells = vec![cell(&["abc"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    let start = Anchor {
        key: EntryKey::cell(&cells[0]),
        index: 0,
        offset: 1,
        row_bias: 0,
    };
    let end = Anchor { offset: 3, ..start };
    let mut selection = Selection::keyboard(view.capture_snapshot(&cells), start);
    selection.set_keyboard_range(start, end);
    view.selection = Some(selection);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('h'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("b"));
    press(&mut view, &cells, KeyCode::Char('h'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("ab"));
}

#[test]
fn vertical_motion_keeps_the_preferred_display_column() {
    let cells = vec![cell(&["abcdef", "x", "abcdef"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    for _ in 0..3 {
        press(&mut view, &cells, KeyCode::Right);
    }
    press(&mut view, &cells, KeyCode::Up);
    press(&mut view, &cells, KeyCode::Up);
    press(&mut view, &cells, KeyCode::Char('v'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("d"));
}

#[test]
fn enter_without_a_mark_exits_but_enter_with_a_mark_copies_and_follows() {
    let cells = vec![cell(&["copy me"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    assert!(matches!(
        view.handle_key(key(KeyCode::Enter), &cells),
        Some(ViewAction::ExitSelectMode)
    ));

    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('v'));
    assert!(matches!(
        view.handle_key(
            KeyEvent::new_with_kind(KeyCode::Char('v'), KeyModifiers::NONE, KeyEventKind::Repeat,),
            &cells,
        ),
        Some(ViewAction::Changed)
    ));
    assert!(matches!(
        view.handle_key(key(KeyCode::Enter), &cells),
        Some(ViewAction::CopyAndFollow(text)) if text == "c"
    ));
}

#[test]
fn word_motion_crosses_entries_and_word_end_selects_the_destination_word() {
    let cells = vec![cell(&["alpha"]), cell(&["beta gamma"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('g'));
    press(&mut view, &cells, KeyCode::Char('j'));

    press(&mut view, &cells, KeyCode::Char('w'));
    press(&mut view, &cells, KeyCode::Char('v'));
    press(&mut view, &cells, KeyCode::Char('e'));

    assert_eq!(view.selected_text(&cells).as_deref(), Some("beta"));
}

#[test]
fn reverse_line_selection_copies_complete_lines_in_document_order() {
    let cells = vec![cell(&["first", "second", "third"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('g'));
    press(&mut view, &cells, KeyCode::Char('j'));
    press(&mut view, &cells, KeyCode::Char('j'));
    press(&mut view, &cells, KeyCode::Char('V'));
    press(&mut view, &cells, KeyCode::Char('k'));

    assert_eq!(view.selected_text(&cells).as_deref(), Some("second\nthird"));
}

#[test]
fn logical_line_end_handles_empty_lines_and_shifted_dollar() {
    let cells = vec![cell(&["abc", "", "z"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('g'));
    assert!(matches!(
        view.handle_key(
            KeyEvent::new(KeyCode::Char('$'), KeyModifiers::SHIFT),
            &cells,
        ),
        Some(ViewAction::Changed)
    ));
    press(&mut view, &cells, KeyCode::Char('v'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("c"));

    press(&mut view, &cells, KeyCode::Esc);
    press(&mut view, &cells, KeyCode::Char('j'));
    press(&mut view, &cells, KeyCode::End);
    press(&mut view, &cells, KeyCode::Char('v'));
    assert_eq!(view.selected_text(&cells), None);

    let cells = vec![cell(&["abc", ""])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('g'));
    press(&mut view, &cells, KeyCode::Char('j'));
    press(&mut view, &cells, KeyCode::End);
}

#[test]
fn transcript_end_character_selection_includes_the_last_grapheme() {
    let cells = vec![cell(&["start"]), cell(&["tail🙂"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('G'));
    press(&mut view, &cells, KeyCode::Char('v'));

    assert_eq!(view.selected_text(&cells).as_deref(), Some("🙂"));
}

#[test]
fn swapping_the_active_end_preserves_text_and_moves_the_shrinking_endpoint() {
    let cells = vec![cell(&["abcdef"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('v'));
    press(&mut view, &cells, KeyCode::Char('o'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("a"));
    press(&mut view, &cells, KeyCode::Char('o'));
    for _ in 0..3 {
        press(&mut view, &cells, KeyCode::Char('l'));
    }
    assert_eq!(view.selected_text(&cells).as_deref(), Some("abcd"));
    press(&mut view, &cells, KeyCode::Char('o'));
    press(&mut view, &cells, KeyCode::Char('l'));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("bcd"));
}

#[test]
fn find_accepts_at_the_match_and_cancel_restores_the_saved_selection() {
    let mut cells = vec![cell(&["alpha beta gamma"])];
    for _ in 0..6 {
        cells.push(cell(&["filler"]));
    }
    cells.push(cell(&["distant"]));
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('/'));
    assert!(view.paste_search("gamma"));
    while view.advance_search(&cells) {}
    press(&mut view, &cells, KeyCode::Enter);
    let accepted_cursor = view
        .select_mode
        .expect("select mode")
        .cursor(view.selection.as_ref().expect("selection"));
    let accepted_position = view.position;

    press(&mut view, &cells, KeyCode::Char('/'));
    assert!(view.paste_search("distant"));
    while view.advance_search(&cells) {}
    render(&mut view, &cells);
    press(&mut view, &cells, KeyCode::Esc);

    assert_eq!(view.position, accepted_position);
    assert_eq!(
        view.select_mode
            .expect("select mode")
            .cursor(view.selection.as_ref().expect("selection")),
        accepted_cursor
    );
    press(&mut view, &cells, KeyCode::Char('v'));
    press(&mut view, &cells, KeyCode::Right);
    assert_eq!(view.selected_text(&cells).as_deref(), Some("ga"));

    press(&mut view, &cells, KeyCode::Char('/'));
    let narrow = Rect::new(0, 0, 8, 6);
    view.render(narrow, &mut Buffer::empty(narrow), &cells);
    press(&mut view, &cells, KeyCode::Esc);
    view.render(narrow, &mut Buffer::empty(narrow), &cells);

    press(&mut view, &cells, KeyCode::Char('/'));
    assert!(matches!(
        view.handle_key(
            KeyEvent::new(KeyCode::Char('\u{3}'), KeyModifiers::NONE),
            &cells,
        ),
        Some(ViewAction::Changed)
    ));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("ga"));

    press(&mut view, &cells, KeyCode::Char('/'));
    assert!(view.paste_search("alpha"));
    while view.advance_search(&cells) {}
    press(&mut view, &cells, KeyCode::Enter);
    assert_eq!(
        view.select_mode
            .expect("select mode")
            .cursor(view.selection.as_ref().expect("selection"))
            .offset,
        0
    );
    assert_eq!(view.selected_text(&cells).as_deref(), Some("alpha beta g"));

    let key = accepted_cursor.key;
    view.select_mode = None;
    view.selection
        .as_mut()
        .expect("selection")
        .set_keyboard_range(
            Anchor {
                offset: 6,
                ..accepted_cursor
            },
            Anchor {
                key,
                offset: 10,
                ..accepted_cursor
            },
        );
    assert!(view.begin_select_mode(&cells));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("beta"));

    for (query, expected_offset, expected_text) in [("\u{301}", 0, "a\u{301}"), ("\nbeta", 4, "b")]
    {
        let cells = vec![cell(&["a\u{301}", "beta"])];
        let mut view = TranscriptView::default();
        render(&mut view, &cells);
        assert!(view.begin_select_mode(&cells));
        press(&mut view, &cells, KeyCode::Char('/'));
        assert!(view.paste_search(query));
        while view.advance_search(&cells) {}
        press(&mut view, &cells, KeyCode::Enter);
        assert_eq!(
            view.select_mode
                .expect("select mode")
                .cursor(view.selection.as_ref().expect("selection"))
                .offset,
            expected_offset
        );
        press(&mut view, &cells, KeyCode::Char('v'));
        assert_eq!(view.selected_text(&cells).as_deref(), Some(expected_text));
    }
}

#[test]
fn the_caret_and_the_mark_are_painted() {
    let cells = vec![cell(&["alpha beta"])];
    let mut view = TranscriptView::default();
    render(&mut view, &cells);
    assert!(view.begin_select_mode(&cells));
    press(&mut view, &cells, KeyCode::Char('0'));
    press(&mut view, &cells, KeyCode::Char('v'));
    press(&mut view, &cells, KeyCode::Char('e'));
    let buffer = render(&mut view, &cells);
    let row = (0..10).map(|x| buffer[(x, 0)].modifier).collect::<Vec<_>>();
    assert!(row[..5]
        .iter()
        .all(|modifier| modifier.contains(ratatui::style::Modifier::REVERSED)));
    assert!(row[4].contains(ratatui::style::Modifier::UNDERLINED));
    assert!(!row[5].contains(ratatui::style::Modifier::REVERSED));
    assert_eq!(view.selected_text(&cells).as_deref(), Some("alpha"));
    assert!(matches!(
        view.handle_key(key(KeyCode::Char('q')), &cells),
        Some(ViewAction::ExitSelectMode)
    ));
    assert!(!view.is_select_mode());
}
