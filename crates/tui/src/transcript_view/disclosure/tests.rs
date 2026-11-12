// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Disclosing an activity's details from the keyboard and the pointer.

use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use hanafubuki_agent::AgentReasoningId;
use ratatui::buffer::Buffer;

use super::*;
use crate::history_cell::{LiveCell, ThinkingCell};
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

fn thinking(item_id: u64, text: &str) -> Arc<dyn HistoryCell> {
    Arc::new(ThinkingCell::replayed(
        item_id,
        text.to_string(),
        2_000,
        false,
    ))
}

/// The row of the screen that shows `needle`, the `nth` one.
fn row_of(frame: &Buffer, needle: &str, nth: usize) -> u16 {
    let area = frame.area;
    (0..area.height)
        .filter(|&y| {
            (0..area.width)
                .map(|x| frame[(x, y)].symbol())
                .collect::<String>()
                .contains(needle)
        })
        .nth(nth)
        .expect("row")
}

#[test]
fn each_thought_expands_alone_by_a_click_on_its_control() {
    let cells = vec![thinking(1, "first plan"), thinking(2, "second plan")];
    let mut view = TranscriptView::default();
    let frame = render(&mut view, &cells, 30, 10);
    let second = row_of(&frame, "+ Show details", 1);
    let click = mouse(MouseEventKind::Down(MouseButton::Left), 5, second);
    assert_eq!(view.handle_mouse(click, &cells), Some(ViewAction::Changed));
    let shown = text(&render(&mut view, &cells, 30, 10));
    assert!(shown.contains("second plan"), "{shown}");
    assert!(!shown.contains("first plan"), "{shown}");
}

#[test]
fn ctrl_o_expands_every_thought_with_text_then_collapses_them() {
    let cells = vec![
        thinking(1, "first plan"),
        tool_call("call-1", 30),
        thinking(2, ""),
        thinking(3, "third plan"),
    ];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 30, 40);
    assert!(view.toggle_thinking(&cells));
    let shown = text(&render(&mut view, &cells, 30, 40));
    assert!(shown.contains("first plan") && shown.contains("third plan"));
    // A thought without text gets no control, and a tool call stays.
    assert_eq!(shown.matches("− Hide details").count(), 2, "{shown}");
    assert!(shown.contains("+ 20 lines"), "{shown}");
    assert!(view.is_following());
    // One collapsed by hand: the next Ctrl-O expands it again.
    let frame = render(&mut view, &cells, 30, 40);
    let first = row_of(&frame, "− Hide details", 0);
    view.handle_mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), 5, first),
        &cells,
    );
    assert!(view.toggle_thinking(&cells));
    let shown = text(&render(&mut view, &cells, 30, 40));
    assert!(shown.contains("first plan") && shown.contains("third plan"));
    assert!(view.toggle_thinking(&cells));
    let hidden = text(&render(&mut view, &cells, 30, 40));
    assert!(!hidden.contains("first plan") && !hidden.contains("third plan"));
    assert!(!view.toggle_thinking(&[tool_call("call-2", 3)]));
}

#[test]
fn a_live_thought_expanded_stays_expanded_when_it_completes() {
    let live = LiveCell::new(ThinkingCell::live(AgentReasoningId::new(9)));
    live.update(|cell| cell.push("streaming plan"));
    let cells: Vec<Arc<dyn HistoryCell>> = vec![cell("before"), live.clone()];
    let mut view = TranscriptView::default();
    render(&mut view, &cells, 30, 10);
    assert!(view.toggle_thinking(&cells));
    assert!(text(&render(&mut view, &cells, 30, 10)).contains("streaming plan"));
    live.update(|cell| {
        cell.push(", done");
        cell.complete(Some(20_790));
    });
    let shown = text(&render(&mut view, &cells, 30, 10));
    assert!(shown.contains("• Thought · 20.7s"), "{shown}");
    assert!(shown.contains("streaming plan, done"), "{shown}");
}

#[test]
fn ctrl_o_keeps_the_entry_at_the_top_of_a_readers_view() {
    let mut cells: Vec<Arc<dyn HistoryCell>> = (0..6)
        .flat_map(|n| {
            let thought = ThinkingCell::replayed(n, "a\nb\nc".to_string(), 1_000 * (n + 1), false);
            [
                cell(format!("entry {n}")),
                Arc::new(thought) as Arc<dyn HistoryCell>,
            ]
        })
        .collect();
    cells.push(cell("last"));
    let first_line = |text: String| {
        text.lines()
            .find(|line| !line.is_empty())
            .unwrap()
            .to_string()
    };
    for rows in [-9, -10, -11] {
        let mut view = TranscriptView::default();
        render(&mut view, &cells, 30, 6);
        view.scroll(&cells, rows);
        let top = first_line(text(&render(&mut view, &cells, 30, 6)));
        assert!(view.toggle_thinking(&cells));
        assert!(!view.is_following());
        assert_eq!(first_line(text(&render(&mut view, &cells, 30, 6))), top);
    }
}
