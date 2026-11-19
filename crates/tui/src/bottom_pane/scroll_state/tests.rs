// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::ScrollState;

#[test]
fn wrap_navigation_and_visibility() {
    let mut s = ScrollState::new();
    let len = 10;
    let vis = 5;

    s.clamp_selection(len);
    assert_eq!(s.selected_idx, Some(0));
    s.ensure_visible(len, vis);
    assert_eq!(s.scroll_top, 0);

    s.move_up_wrap(len);
    s.ensure_visible(len, vis);
    assert_eq!(s.selected_idx, Some(len - 1));
    match s.selected_idx {
        Some(sel) => assert!(s.scroll_top <= sel),
        None => panic!("expected Some(selected_idx) after wrap"),
    }

    s.move_down_wrap(len);
    s.ensure_visible(len, vis);
    assert_eq!(s.selected_idx, Some(0));
    assert_eq!(s.scroll_top, 0);
}

#[test]
fn moving_by_stops_at_the_ends() {
    let mut s = ScrollState::new();
    let len = 20;
    let vis = 8;
    s.clamp_selection(len);

    // Down a page, and at the last row it stays there instead of wrapping.
    s.move_by(len, 8);
    assert_eq!(s.selected_idx, Some(8));
    s.move_by(len, 8);
    s.move_by(len, 8);
    assert_eq!(s.selected_idx, Some(len - 1));

    // Up a page, and at the first row it stays there.
    s.move_by(len, -8);
    assert_eq!(s.selected_idx, Some(len - 9));
    s.move_by(len, -8);
    s.move_by(len, -8);
    assert_eq!(s.selected_idx, Some(0));

    // The window follows the page.
    s.move_by(len, 8);
    s.ensure_visible(len, vis);
    assert_eq!(s.scroll_top, 1);

    // An empty list has nothing to select.
    s.move_by(0, 8);
    assert_eq!(s.selected_idx, None);
    assert_eq!(s.scroll_top, 0);
}
