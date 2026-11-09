// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Literal search, its continuation into older history, and the
//! restoring of the presentation it started from share source offsets.

use ratatui::style::Modifier;

use super::scan::{find_literal, SCAN_BYTES};
use super::*;
use crate::transcript_view::tests::fixtures::{cell, ctrl, key, render, text, tool_call};

fn finish_scan(view: &mut TranscriptView, cells: &[Arc<dyn HistoryCell>]) {
    for _ in 0..512 {
        if !view.advance_search(cells) {
            return;
        }
    }
    panic!("search did not settle within bounded frames");
}

fn match_index(view: &TranscriptView) -> Option<usize> {
    view.search.current.as_ref().map(|found| found.anchor.index)
}

#[test]
fn find_keeps_context_and_allows_reading_without_losing_the_query() {
    let cells = vec![
        cell("earlier needle"),
        cell("explanation before the result"),
        cell("needle in the result"),
        cell("explanation after the result"),
        cell("latest message"),
    ];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 40, 6);
    view.handle_key(key(KeyCode::F(3)), &cells);
    assert!(view.is_search_editing());
    view.paste_search("needle");
    finish_scan(&mut view, &cells);
    let found = render(&mut view, &cells, 40, 6);
    insta::assert_snapshot!(text(&found), @r"
    explanation before the result

    needle in the result

    explanation after the result
    ");
    assert!(found[(0, 2)].modifier.contains(Modifier::REVERSED));

    let matched = view.position;
    view.handle_key(key(KeyCode::PageDown), &cells);
    assert_eq!(
        (view.is_following(), view.search.editor.text()),
        (true, "needle")
    );
    view.handle_key(key(KeyCode::PageUp), &cells);
    finish_scan(&mut view, &cells);
    let scrolled = view.position;
    assert_ne!(scrolled, matched);
    view.handle_key(key(KeyCode::Enter), &cells);
    assert_eq!(
        (
            view.position,
            view.is_search_editing(),
            view.has_active_interaction()
        ),
        (scrolled, false, true),
    );
    // A held Enter stays in search.
    assert_eq!(
        view.handle_key(key(KeyCode::Enter), &cells),
        Some(ViewAction::Changed)
    );
    view.begin_search();
    assert_eq!(
        (view.position, view.search.editor.text()),
        (scrolled, "needle")
    );
    view.handle_key(key(KeyCode::Enter), &cells);
    view.handle_key(ctrl('p'), &cells);
    finish_scan(&mut view, &cells);
    assert_eq!(match_index(&view), Some(0));
    view.handle_key(key(KeyCode::Esc), &cells);
    assert_eq!(
        (
            view.is_following(),
            view.is_detailed(),
            view.has_active_interaction()
        ),
        (true, false, false),
    );
}

#[test]
fn find_reduces_context_to_keep_a_tall_match_visible() {
    let cells = vec![
        cell("earlier"),
        cell("one two three four five"),
        cell("later"),
    ];
    let mut frames = Vec::new();
    for height in [6, 5, 3] {
        let mut view = TranscriptView::default();
        render(&mut view, &cells, 6, height);
        view.begin_search();
        view.paste_search("one two three four five");
        finish_scan(&mut view, &cells);
        frames.push(format!(
            "height {height}\n{}",
            text(&render(&mut view, &cells, 6, height))
        ));
    }
    insta::assert_snapshot!(frames.join("\n"), @r"
    height 6

    one
    two
    three
    four
    five
    height 5
    one
    two
    three
    four
    five
    height 3
    one
    two
    three
    ");
}

#[test]
fn literal_search_preserves_unicode_source_offsets() {
    let text = "界 CAFÉ [x].* İ";
    assert_eq!(
        [
            find_literal(text, "café", Direction::Newer),
            find_literal(text, "[x].*", Direction::Older),
            find_literal(text, "i\u{307}", Direction::Newer),
            find_literal(text, "missing", Direction::Older),
        ],
        [Some(4..9), Some(10..15), Some(16..18), None],
    );
}

#[test]
fn scanning_yields_and_finds_a_match_crossing_a_chunk_boundary() {
    let text = format!(
        "{}nEeDlE{}",
        "a".repeat(SCAN_BYTES - 2),
        "z".repeat(SCAN_BYTES * 2)
    );
    let cells = vec![cell(text)];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 80, 4);
    view.begin_search();
    view.paste_search("needle");
    assert!(view.advance_search(&cells));
    assert!(view.search.current.is_none());
    view.prepare_width(40);
    finish_scan(&mut view, &cells);
    let found = view.search.current.as_ref().expect("match across chunks");
    assert_eq!(
        (found.anchor.offset, found.end),
        (SCAN_BYTES - 2, SCAN_BYTES + 4)
    );
}

#[test]
fn short_history_entries_share_a_frame_without_starving_input() {
    let mut cells = vec![cell("needle in the oldest message")];
    cells.extend((0..64).map(|index| cell(format!("newer message {index}"))));
    let mut view = TranscriptView::default();
    view.begin_search();
    view.paste_search("needle");
    let mut frames = 1;
    while view.advance_search(&cells) {
        frames += 1;
        assert!(frames < 16, "short entries cost a frame each");
    }
    assert!(frames > 1, "a long history yields to input");
    let found = view.search.current.as_ref().expect("oldest match");
    assert_eq!(
        (found.anchor.index, found.anchor.offset..found.end),
        (0, 0..6)
    );
}

#[test]
fn search_resumes_in_a_loaded_page_without_rescanning_the_newer_entry() {
    let newest = cell("newer messages");
    let mut cells = vec![Arc::clone(&newest)];
    let mut view = TranscriptView {
        history: TranscriptHistoryState::Partial,
        ..TranscriptView::default()
    };
    view.begin_search();
    view.paste_search("needle");
    finish_scan(&mut view, &cells);
    assert!(view.needs_history(&cells));
    view.history = TranscriptHistoryState::LoadingOlder;
    assert!(!view.advance_search(&cells));
    cells.insert(0, cell("old NEEDLE"));
    view.history = TranscriptHistoryState::Complete;
    view.history_loaded(&cells, 0..1);
    finish_scan(&mut view, &cells);
    let found = view.search.current.as_ref().expect("match in the page");
    assert_eq!(
        (
            found.anchor.key,
            found.anchor.index,
            found.anchor.offset,
            found.end
        ),
        (EntryKey::cell(&cells[0]), 0, 4, 10)
    );
    assert!(!view.needs_history(&cells));
}

#[test]
fn next_previous_highlight_and_cancel_share_the_transcript_position() {
    let cells = vec![cell("Needle one"), cell("Needle two")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 3);
    view.jump_to_entry(&cells, 0);
    let original = view.position;
    view.begin_search();
    view.paste_search("needle");
    finish_scan(&mut view, &cells);
    assert_eq!(match_index(&view), Some(1));
    view.handle_key(ctrl('p'), &cells);
    finish_scan(&mut view, &cells);
    assert_eq!(match_index(&view), Some(0));
    view.handle_key(ctrl('n'), &cells);
    finish_scan(&mut view, &cells);
    let buffer = render(&mut view, &cells, 20, 3);
    assert_eq!(
        (0..10)
            .map(|column| buffer[(column, 1)].modifier.contains(Modifier::REVERSED))
            .collect::<Vec<_>>(),
        vec![true, true, true, true, true, true, false, false, false, false]
    );
    view.handle_key(key(KeyCode::Left), &cells);
    assert!(matches!(view.search.progress, Progress::Found));
    view.handle_key(key(KeyCode::Esc), &cells);
    assert_eq!(
        (
            view.position,
            view.search.is_active(),
            view.search.editor.text()
        ),
        (original, false, ""),
    );
}

#[test]
fn searching_on_stops_at_the_ends_without_wrapping() {
    let cells = vec![
        cell("SHORT T0000 USER]"),
        cell("SHORT T0001 USER]"),
        cell("SHORT T0002 USER]"),
    ];
    let mut view = TranscriptView::default();
    view.begin_search();
    view.paste_search("USER]");
    finish_scan(&mut view, &cells);
    let mut positions = Vec::new();
    for character in ['p', 'p', 'p', 'n', 'n', 'n'] {
        view.handle_key(ctrl(character), &cells);
        finish_scan(&mut view, &cells);
        positions.push((
            match_index(&view).unwrap(),
            matches!(view.search.progress, Progress::Exhausted),
        ));
    }
    assert_eq!(
        positions,
        vec![
            (1, false),
            (0, false),
            (0, true),
            (1, false),
            (2, false),
            (2, true)
        ],
    );
}

#[test]
fn find_reveals_a_collapsed_activity_only_while_it_is_the_match() {
    let cells = vec![tool_call("call-1", 30), cell("latest")];
    let mut view = TranscriptView::default();
    let compact = text(&render(&mut view, &cells, 30, 12));
    assert!(compact.contains("+ 20 lines"));
    view.begin_search();
    view.paste_search("out 15");
    finish_scan(&mut view, &cells);
    let found = text(&render(&mut view, &cells, 30, 12));
    assert!(found.contains("out 15"), "{found}");
    view.handle_key(key(KeyCode::Esc), &cells);
    assert_eq!(text(&render(&mut view, &cells, 30, 12)), compact);
}

#[test]
fn search_keeps_both_presentations_positions() {
    let cells = vec![
        cell("old needle"),
        cell("saved detailed position"),
        cell("saved compact position"),
    ];
    for detailed in [false, true] {
        let mut view = TranscriptView::default();
        render(&mut view, &cells, 30, 2);
        view.set_presentation(true, HistoryRenderMode::Rich);
        view.jump_to_entry(&cells, 1);
        let detailed_position = view.position;
        view.set_presentation(false, HistoryRenderMode::Rich);
        view.jump_to_entry(&cells, 2);
        let compact_position = view.position;
        view.set_presentation(detailed, HistoryRenderMode::Rich);
        let positions = (view.position, view.saved_position);
        view.begin_search();
        finish_scan(&mut view, &cells);
        assert_eq!((view.position, view.saved_position), positions);
        view.paste_search("needle");
        finish_scan(&mut view, &cells);
        assert_eq!(match_index(&view), Some(0));
        view.handle_key(ctrl('c'), &cells);
        assert_eq!((view.position, view.saved_position), positions);
        view.set_presentation(!detailed, HistoryRenderMode::Rich);
        let expected = if detailed {
            compact_position
        } else {
            detailed_position
        };
        assert_eq!(view.position, expected);
    }
}

#[test]
fn a_live_scan_restarts_when_its_source_changes() {
    let mut view = TranscriptView::default();
    render(&mut view, &[], 80, 4);
    view.begin_search();
    let retired = format!("needle{}", "z".repeat(SCAN_BYTES * 2));
    view.sync_live_tail(80, None, |_| Some(vec![Line::from(retired)]));
    view.paste_search("needle");
    assert!(view.advance_search(&[]));
    view.sync_live_tail(80, None, |_| Some(vec![Line::from("new needle")]));
    finish_scan(&mut view, &[]);
    let found = view.search.current.as_ref().expect("live match");
    assert_eq!(found.anchor.offset..found.end, 4..10);
}

#[test]
fn a_pasted_query_is_bounded_to_whole_graphemes() {
    let mut view = TranscriptView::default();
    view.begin_search();
    view.paste_search("X\r\nY\rZ");
    assert_eq!(view.search.editor.text(), "X\nY\nZ");
    view.cancel_search();
    view.begin_search();
    let original = format!("{}Z", "a".repeat(QUERY_BYTES - 2));
    view.paste_search(&original);
    view.handle_key(key(KeyCode::Left), &[]);
    view.paste_search("e\u{301}");
    assert_eq!(
        (view.search.editor.text(), view.search.query_truncated),
        (original.as_str(), true)
    );
    view.paste_search("b");
    let full = format!("{}bZ", "a".repeat(QUERY_BYTES - 2));
    view.handle_key(key(KeyCode::Char('c')), &[]);
    assert_eq!(
        (view.search.editor.text(), view.search.query_truncated),
        (full.as_str(), true)
    );
}

#[test]
fn selection_keys_come_before_find_and_typing_returns_to_the_query() {
    let cells = vec![cell("needle")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 3);
    view.begin_search();
    view.paste_search("needle");
    finish_scan(&mut view, &cells);
    render(&mut view, &cells, 20, 3);
    view.handle_key(ctrl(' '), &cells);
    view.handle_key(key(KeyCode::Right), &cells);
    assert!(matches!(
        view.handle_key(ctrl('c'), &cells),
        Some(ViewAction::Copy(_))
    ));
    assert!(view.is_search_editing());
    view.handle_key(key(KeyCode::Char('x')), &cells);
    assert_eq!(
        (view.selection.is_none(), view.search.editor.text()),
        (true, "needlex")
    );
}
