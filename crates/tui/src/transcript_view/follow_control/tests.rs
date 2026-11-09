// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The follow gestures and the return control share one reading state.

use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::*;
use crate::transcript_view::tests::fixtures::{cell, text};

fn paint(view: &mut TranscriptView, cells: &[Arc<dyn HistoryCell>], width: u16) -> Buffer {
    let area = Rect::new(0, 0, width, 4);
    let mut buf = Buffer::empty(area);
    view.render(Rect::new(0, 0, width, 3), &mut buf, cells);
    view.render_follow_control(Some(Rect::new(0, 3, width, 1)), &mut buf);
    buf
}

fn mouse(
    view: &mut TranscriptView,
    cells: &[Arc<dyn HistoryCell>],
    kind: MouseEventKind,
    column: u16,
    row: u16,
) {
    view.handle_mouse(
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        cells,
    );
}

#[test]
fn an_empty_click_restores_following_but_not_an_existing_pause() {
    for following in [true, false] {
        let mut cells = vec![cell("one\ntwo\nthree\nfour")];
        let mut view = TranscriptView::default();
        paint(&mut view, &cells, 40);
        if !following {
            view.scroll(&cells, -1);
            paint(&mut view, &cells, 40);
        }
        mouse(
            &mut view,
            &cells,
            MouseEventKind::Down(MouseButton::Left),
            0,
            0,
        );
        cells.push(cell("arrived during click"));
        paint(&mut view, &cells, 40);
        mouse(
            &mut view,
            &cells,
            MouseEventKind::Up(MouseButton::Left),
            0,
            0,
        );
        assert_eq!(
            (view.is_following(), view.selected_text(&cells)),
            (following, None)
        );
    }
}

#[test]
fn a_no_op_scroll_keeps_following_and_a_real_scroll_pauses() {
    let cells = vec![cell("one\ntwo")];
    let mut view = TranscriptView::default();
    paint(&mut view, &cells, 40);
    view.scroll(&cells, -3);
    assert!(view.is_following());
    // Toward an unloaded page, scrolling means something even when the
    // loaded rows fit.
    view.history = TranscriptHistoryState::Partial;
    view.scroll(&cells, -3);
    assert!(!view.is_following());
    assert!(view.needs_history(&cells));
    view.history = TranscriptHistoryState::Complete;
    view.jump_to_latest();
    let cells = vec![cell("one\ntwo\nthree\nfour")];
    paint(&mut view, &cells, 40);
    view.scroll(&cells, -3);
    assert!(view.can_return_to_latest());
    view.scroll(&cells, 3);
    assert!(view.is_following());
}

#[test]
fn an_empty_viewport_does_not_show_the_control() {
    let cells = vec![cell("one")];
    let mut view = TranscriptView::default();
    let gap = Rect::new(0, 0, 40, 1);
    let mut buffer = Buffer::empty(gap);
    view.render(Rect { height: 0, ..gap }, &mut buffer, &cells);
    view.render_follow_control(Some(gap), &mut buffer);
    assert!(view.is_following());
    assert!(view.follow_control.area.is_none());
    assert_eq!(text(&buffer), "");
}

#[test]
fn the_control_tracks_pause_activity_hover_resize_and_click() {
    let mut cells = vec![cell("one\ntwo\nthree\nfour\nfive")];
    let mut view = TranscriptView::default();
    paint(&mut view, &cells, 80);
    assert!(view.follow_control.area.is_none());
    view.scroll(&cells, -2);
    let mut frames = vec![format!("paused\n{}", text(&paint(&mut view, &cells, 80)))];
    cells.push(cell("new output"));
    for width in [80, 20] {
        frames.push(format!(
            "activity at {width}\n{}",
            text(&paint(&mut view, &cells, width))
        ));
    }
    let area = view.follow_control.area.unwrap();
    mouse(&mut view, &cells, MouseEventKind::Moved, area.x, area.y);
    let hovered = paint(&mut view, &cells, 20);
    assert!(hovered[(area.x, area.y)]
        .modifier
        .contains(ratatui::style::Modifier::REVERSED));
    mouse(
        &mut view,
        &cells,
        MouseEventKind::Down(MouseButton::Left),
        area.x,
        area.y,
    );
    mouse(
        &mut view,
        &cells,
        MouseEventKind::Up(MouseButton::Left),
        area.x,
        area.y,
    );
    assert!(view.is_following());
    paint(&mut view, &cells, 20);
    assert!(view.follow_control.area.is_none());
    insta::assert_snapshot!(frames.join("\n\n"), @r"
    paused
    one
    two
    three
                                 ↓ Back to bottom · esc

    activity at 80
    one
    two
    three
                         New activity · ↓ Back to bottom · esc

    activity at 20
    one
    two
    three
       New · ↓ Bottom
    ");
}

#[test]
fn a_hidden_control_does_not_take_clicks_and_a_drag_out_cancels() {
    let cells = vec![cell("one\ntwo\nthree\nfour")];
    let mut view = TranscriptView::default();
    paint(&mut view, &cells, 40);
    view.scroll(&cells, -1);
    paint(&mut view, &cells, 40);
    let area = view.follow_control.area.unwrap();
    mouse(
        &mut view,
        &cells,
        MouseEventKind::Down(MouseButton::Left),
        area.x,
        area.y,
    );
    mouse(
        &mut view,
        &cells,
        MouseEventKind::Up(MouseButton::Left),
        0,
        0,
    );
    assert!(!view.is_following());
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
    view.render_follow_control(None, &mut buf);
    assert!(view
        .handle_follow_control_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        })
        .is_none());
}

#[test]
fn height_changes_backfill_after_an_empty_click_but_keep_a_deliberate_pause() {
    let rows = (0..60)
        .map(|row| format!("row {row:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let cells = vec![cell(rows)];
    for pause in ["none", "scroll", "selection"] {
        let mut view = TranscriptView::default();
        let short = Rect::new(0, 0, 120, 24);
        view.render(short, &mut Buffer::empty(short), &cells);
        if pause == "scroll" {
            view.scroll(&cells, -6);
            view.render(short, &mut Buffer::empty(short), &cells);
        }
        if pause == "selection" {
            view.begin_selection(&cells, 0, 0, 3);
        } else {
            mouse(
                &mut view,
                &cells,
                MouseEventKind::Down(MouseButton::Left),
                0,
                0,
            );
            mouse(
                &mut view,
                &cells,
                MouseEventKind::Up(MouseButton::Left),
                0,
                0,
            );
        }
        let before = (view.visible[0].key, view.visible[0].row);
        let selected = view.selected_text(&cells);
        for height in [50, 24, 50] {
            let area = Rect::new(0, 0, 120, height);
            view.render(area, &mut Buffer::empty(area), &cells);
            assert_eq!(view.is_following(), pause == "none");
            assert_eq!(view.selected_text(&cells), selected);
            if pause == "none" {
                assert_eq!(view.visible.len(), usize::from(height));
                assert_eq!(view.visible.last().unwrap().row, 59);
            } else {
                assert_eq!((view.visible[0].key, view.visible[0].row), before);
            }
        }
    }
}
