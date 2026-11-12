// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The viewport across scrolling, new output, prepended history, resizes,
//! entries that change in place and the two presentations.

pub(crate) mod fixtures;

use std::sync::atomic::{AtomicUsize, Ordering};

use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use super::*;
use crate::history_cell::{new_tool_call, LiveCell, ToolCallCell};
use fixtures::{cell, ctrl, find, key, live, mouse, prompt, render, text, TestCell};

#[test]
fn following_shows_the_latest_rows_with_a_blank_row_between_entries() {
    let cells = vec![cell("first"), cell("second\nthird")];
    let mut view = TranscriptView::default();
    assert_eq!(text(&render(&mut view, &cells, 20, 3)), "\nsecond\nthird");
    assert!(view.is_following());
}

#[test]
fn page_keys_scroll_a_page_less_one_row_and_return() {
    let cells = [cell(
        (1..=30)
            .map(|row| format!("row {row}\n"))
            .collect::<String>(),
    )];
    let mut view = TranscriptView::default();
    let latest = render(&mut view, &cells, 20, 8);
    assert_eq!(
        view.handle_key(key(KeyCode::PageUp), &cells),
        Some(ViewAction::Changed)
    );
    insta::assert_snapshot!(text(&render(&mut view, &cells, 20, 8)), @r"
    row 16
    row 17
    row 18
    row 19
    row 20
    row 21
    row 22
    row 23
    ");
    view.handle_key(key(KeyCode::PageDown), &cells);
    assert_eq!(render(&mut view, &cells, 20, 8), latest);
    assert!(view.is_following());
    // Typing is the composer's.
    for typed in [key(KeyCode::Char('j')), ctrl('u')] {
        assert_eq!(view.handle_key(typed, &cells), None);
    }
}

#[test]
fn empty_entries_add_no_separator_but_visible_ones_do() {
    let cells = vec![
        cell(""),
        cell(""),
        cell("First"),
        cell(""),
        cell("Second"),
        live("Live"),
    ];
    let mut view = TranscriptView::default();
    let frame = render(&mut view, &cells, 20, 5);
    insta::assert_snapshot!(text(&frame), @r"
    First

    Second

    Live
    ");
    // Scrolling past the first entry stops at its text, not a separator.
    view.scroll(&cells, -100);
    assert_eq!(render(&mut view, &cells, 20, 5), frame);
}

#[test]
fn reading_survives_prepend_and_new_output_then_returns_to_latest() {
    let mut cells = vec![cell("older\nline two"), cell("current\nlast line")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 3);
    view.scroll(&cells, -2);
    let before = text(&render(&mut view, &cells, 20, 3));
    cells.insert(0, cell("loaded from history"));
    cells.push(cell("new output"));
    assert_eq!(text(&render(&mut view, &cells, 20, 3)), before);
    assert!(view.unseen_activity);
    view.jump_to_latest();
    insta::assert_snapshot!(text(&render(&mut view, &cells, 20, 3)), @r"
    last line

    new output
    ");
}

#[test]
fn a_loaded_page_moves_the_reading_anchor_before_the_next_frame() {
    let mut cells = vec![cell("current first"), cell("current last")];
    let mut view = TranscriptView::default();
    view.jump_to_entry(&cells, 0);
    assert!(view.near_start(&cells));
    let earlier = (0..100)
        .map(|index| cell(format!("earlier {index}")))
        .collect::<Vec<_>>();
    cells.splice(0..0, earlier);
    view.history_loaded(&cells, 0..100);
    assert!(!view.needs_history(&cells));
    let Position::Reading(anchor) = view.position else {
        panic!("reading anchor");
    };
    assert_eq!(
        (anchor.key, anchor.index),
        (EntryKey::cell(&cells[100]), 100)
    );
}

#[test]
fn the_beginning_waits_for_older_pages_with_the_view_held() {
    let running = live("live zero\nlive one");
    let mut cells = vec![cell("previous answer"), running.clone()];
    let mut view = TranscriptView {
        history: TranscriptHistoryState::Partial,
        ..TranscriptView::default()
    };
    let before = render(&mut view, &cells, 20, 2);
    view.jump_to_beginning(&cells);
    assert_eq!(view.history, TranscriptHistoryState::LoadingBeginning);
    cells.insert(0, cell("older page"));
    view.history_loaded(&cells, 0..1);
    running.update(|activity| activity.text = "new output".to_owned());
    assert_eq!(render(&mut view, &cells, 20, 2), before);
    // A navigation of the reader's own supersedes the jump.
    view.scroll(&cells, -1);
    assert_eq!(view.history, TranscriptHistoryState::LoadingOlder);
}

#[test]
fn single_row_scrolling_crosses_an_entry_separator_in_both_directions() {
    let cells = vec![cell("first"), cell("second")];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 1);
    let mut frames = Vec::new();
    for rows in [-1, -1, 1, 1] {
        view.scroll(&cells, rows);
        frames.push(text(&render(&mut view, &cells, 20, 1)));
    }
    assert_eq!(frames, ["", "first", "", "second"]);
}

#[test]
fn only_visible_history_is_laid_out_and_repeated_frames_reuse_it() {
    let renders = Arc::new(AtomicUsize::new(0));
    let cells = (0..10_000)
        .map(|index| {
            Arc::new(TestCell {
                text: format!("message {index}"),
                renders: Arc::clone(&renders),
            }) as Arc<dyn HistoryCell>
        })
        .collect::<Vec<_>>();
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 40, 20);
    let initial = renders.load(Ordering::Relaxed);
    assert!(initial <= 12, "laid out {initial} entries for 20 rows");
    render(&mut view, &cells, 40, 20);
    assert_eq!(renders.load(Ordering::Relaxed), initial);
}

#[test]
fn resize_keeps_the_reading_anchor() {
    let mut cells = vec![cell(
        "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda",
    )];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 10, 2);
    view.scroll(&cells, -2);
    let Position::Reading(anchor) = view.position else {
        panic!("reading anchor");
    };
    render(&mut view, &cells, 20, 2);
    assert_eq!(view.position, Position::Reading(anchor));
    assert!(!view.unseen_activity);
    cells.push(cell("new output"));
    render(&mut view, &cells, 20, 2);
    assert!(view.unseen_activity);
}

#[test]
fn a_zero_sized_viewport_is_safe_and_empty_history_accepts_navigation() {
    let mut view = TranscriptView::default();
    render(&mut view, &[], 0, 0);
    view.scroll(&[], -10);
    view.scroll(&[], 10);
    assert_eq!(view.selected_text(&[]), None);
    assert_eq!(
        view.handle_key(key(KeyCode::PageUp), &[]),
        Some(ViewAction::Changed)
    );
}

#[derive(Debug)]
struct VersionedCell {
    version: usize,
    renders: Arc<AtomicUsize>,
}

impl HistoryCell for VersionedCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.renders.fetch_add(1, Ordering::Relaxed);
        (0..8)
            .map(|row| Line::from(format!("version {} row {row}", self.version)))
            .collect()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.display_lines(80)
    }
}

#[test]
fn an_entry_that_changes_in_place_is_laid_out_again_only_at_a_new_revision() {
    let renders = Arc::new(AtomicUsize::new(0));
    let entry = LiveCell::new(VersionedCell {
        version: 1,
        renders: Arc::clone(&renders),
    });
    let cells: Vec<Arc<dyn HistoryCell>> = vec![cell("before"), entry.clone()];
    let mut view = TranscriptView::default();
    let first = render(&mut view, &cells, 24, 5);
    render(&mut view, &cells, 24, 5);
    assert_eq!(renders.load(Ordering::Relaxed), 1);
    entry.update(|cell| cell.version = 2);
    let second = render(&mut view, &cells, 24, 5);
    assert_eq!(renders.load(Ordering::Relaxed), 2);
    assert!(text(&first).contains("version 1 row 3"));
    insta::assert_snapshot!(text(&second), @r"
    version 2 row 3
    version 2 row 4
    version 2 row 5
    version 2 row 6
    version 2 row 7
    ");
    find(&mut view, &cells, "version 2");
    let matched = text(&render(&mut view, &cells, 24, 5));
    // The match is read in the revision it was found in.
    entry.update(|cell| cell.version = 3);
    assert_eq!(text(&render(&mut view, &cells, 24, 5)), matched);
}

#[test]
fn a_reader_in_a_live_entry_keeps_its_revision_through_completion_and_rejoins_latest() {
    let running = live("live zero\nlive one\nlive two\nlive three\nlive four\nlive five");
    let mut cells = vec![cell("before"), running.clone()];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 2);
    view.scroll(&cells, -2);
    let before = text(&render(&mut view, &cells, 20, 2));
    let reading = view.position;
    running.update(|activity| {
        activity.text.push_str("\nlive six");
        activity.done = true;
    });
    assert_eq!(text(&render(&mut view, &cells, 20, 2)), before);
    cells.push(live("new entry"));
    assert_eq!(text(&render(&mut view, &cells, 20, 2)), before);
    render(&mut view, &cells, 9, 2);
    assert_eq!(view.position, reading);
    assert!(view.unseen_activity);
    view.begin_selection(&cells, 0, 0, 3);
    assert_eq!(view.selected_text(&cells), Some("live two\n".to_string()));
    view.handle_key(key(KeyCode::Esc), &cells);
    assert_eq!(text(&render(&mut view, &cells, 20, 2)), before);
    view.jump_to_latest();
    assert!(view.snapshot().is_none());
    insta::assert_snapshot!(text(&render(&mut view, &cells, 20, 2)), @r"

    new entry
    ");
}

#[test]
fn scrolling_out_of_a_held_live_entry_releases_its_revision() {
    let running = live("zero\none\ntwo\nthree\nfour\nfive");
    let cells = vec![cell("before"), running.clone()];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 2);
    view.scroll(&cells, -2);
    assert!(view.held_reading.is_some());
    running.update(|activity| {
        activity.text.push_str("\nsix");
        activity.done = true;
    });
    view.scroll(&cells, -100);
    assert!(view.held_reading.is_none());
    view.scroll(&cells, 100);
    assert!(view.is_following());
    assert!(text(&render(&mut view, &cells, 20, 2)).contains("six"));
}

#[test]
fn a_change_of_the_last_entry_below_a_reader_is_unseen_activity() {
    let running = live("running");
    let cells = vec![cell("one\ntwo\nthree"), running.clone()];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 2);
    view.scroll(&cells, -3);
    render(&mut view, &cells, 20, 2);
    assert!(!view.unseen_activity);
    running.update(|activity| activity.done = true);
    render(&mut view, &cells, 20, 2);
    assert!(view.unseen_activity);
}

#[test]
fn older_history_is_wanted_near_the_start_counting_earlier_entries() {
    let cells = (0..6)
        .map(|index| cell(format!("row {index}")))
        .collect::<Vec<_>>();
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 3);
    assert!(!view.needs_history(&cells));
    view.scroll(&cells, -6);
    assert!(view.needs_history(&cells));
}

#[test]
fn the_detailed_presentation_keeps_its_own_reading_position() {
    let cells = (0..10)
        .map(|index| cell(format!("entry {index}")))
        .collect::<Vec<_>>();
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 3);
    view.scroll(&cells, -6);
    let compact = text(&render(&mut view, &cells, 20, 3));
    // The first switch reads on where the other presentation was.
    view.set_presentation(true, HistoryRenderMode::Rich);
    assert!(view.is_detailed());
    assert!(!view.is_following());
    view.jump_to_latest();
    view.set_presentation(false, HistoryRenderMode::Rich);
    assert_eq!(text(&render(&mut view, &cells, 20, 3)), compact);
    view.set_presentation(true, HistoryRenderMode::Rich);
    assert!(view.is_following());
}

#[test]
fn the_detailed_presentation_shows_thinking_the_compact_one_collapses() {
    let thinking: Arc<dyn HistoryCell> = Arc::new(crate::history_cell::ThinkingCell::replayed(
        1,
        "the plan".to_string(),
        2000,
        false,
    ));
    let cells = vec![prompt("hi"), thinking];
    let mut view = TranscriptView::default();
    let compact = text(&render(&mut view, &cells, 30, 6));
    assert!(!compact.contains("the plan"));
    view.set_presentation(true, HistoryRenderMode::Rich);
    assert!(text(&render(&mut view, &cells, 30, 6)).contains("the plan"));
    view.set_presentation(false, HistoryRenderMode::Raw);
    insta::assert_snapshot!(text(&render(&mut view, &cells, 30, 6)), @r"
    hi

    • Thought · 2.0s
    the plan
    ");
}

#[test]
fn a_user_prompt_brings_its_own_spacing() {
    let cells = vec![cell("answer"), prompt("question")];
    let mut view = TranscriptView::default();
    insta::assert_snapshot!(text(&render(&mut view, &cells, 20, 4)), @r"
    answer

    › question
    ");
}

/// A shell call whose output is `lines` numbered lines.
fn complete(call: &LiveCell<ToolCallCell>, lines: usize) {
    call.update(|cell| {
        cell.complete(
            false,
            crate::history_cell::ToolResultView {
                text: Some((1..=lines).map(|line| format!("out {line}\n")).collect()),
                exit_code: Some(0),
                ..Default::default()
            },
        )
    });
}

#[test]
fn a_tool_call_changes_in_place_and_keeps_its_disclosure_as_it_changes() {
    let call = LiveCell::new(new_tool_call("call-1".into(), "shell".into(), None));
    let cells = vec![cell("before"), call.clone()];
    let mut view = TranscriptView::default();
    assert!(text(&render(&mut view, &cells, 30, 16)).contains("Running shell"));
    complete(&call, 30);
    let frame = text(&render(&mut view, &cells, 30, 16));
    assert!(frame.contains("Called shell"), "{frame}");
    assert!(frame.contains("+ 20 lines"), "{frame}");
    assert!(!frame.contains("out 6\n"));
    view.handle_key(key(KeyCode::F(4)), &cells);
    view.handle_key(key(KeyCode::Enter), &cells);
    // A later revision of the entry keeps it expanded.
    complete(&call, 31);
    view.jump_to_latest();
    let frame = text(&render(&mut view, &cells, 30, 40));
    assert!(frame.contains("− Show less"), "{frame}");
    assert!(frame.contains("out 31"), "{frame}");
}

#[test]
fn the_wheel_scrolls_three_rows() {
    let cells = [cell(
        (1..=20)
            .map(|row| format!("row {row}\n"))
            .collect::<String>(),
    )];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 4);
    view.handle_mouse(mouse(MouseEventKind::ScrollUp, 0, 0), &cells);
    assert_eq!(
        text(&render(&mut view, &cells, 20, 4)),
        "row 14\nrow 15\nrow 16\nrow 17"
    );
    view.handle_mouse(mouse(MouseEventKind::ScrollDown, 0, 0), &cells);
    assert!(view.is_following());
    assert_eq!(
        view.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0, 9), &cells),
        None
    );
}

#[test]
fn a_selection_keeps_a_live_entry_below_the_view_as_it_was() {
    let running = live("live one\nlive two");
    let cells = vec![cell("a\nb\nc\nd"), running.clone()];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 20, 2);
    view.scroll(&cells, -100);
    assert!(text(&render(&mut view, &cells, 20, 2)).starts_with('a'));
    view.begin_selection(&cells, 0, 0, 1);
    running.update(|activity| {
        activity.text = "done".to_owned();
        activity.done = true;
    });
    view.scroll(&cells, 100);
    render(&mut view, &cells, 20, 2);
    view.extend_selection(19, 1);
    let selected = view.selected_text(&cells).unwrap_or_default();
    assert!(selected.ends_with("live two"), "{selected:?}");
}
