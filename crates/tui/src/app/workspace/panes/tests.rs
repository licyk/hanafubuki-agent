use super::*;

#[test]
fn a_resized_sidebar_keeps_its_share_and_restores_the_exact_dragged_column() {
    let mut workspace = Workspace::default();
    workspace.set_side_width(211, 87);
    let columns = |total| {
        workspace
            .split_panes(Rect::new(0, 0, total, 24))
            .map(|(_, side, _)| side.width)
    };

    assert_eq!(columns(211), Some(87));
    assert_eq!(columns(300), Some(123));
    assert_eq!(columns(101), Some(36));
    assert_eq!(columns(99), None);
    assert_eq!(columns(211), Some(87));
}

/// The side pane slides open: part-way through, less than its full width is
/// drawn, and the main pane takes the rest.
#[test]
fn a_sliding_side_pane_takes_the_share_of_the_width_it_has_reached() {
    let workspace = Workspace::default();
    let screen = Rect::new(0, 0, 120, 24);
    let side = |fraction| {
        workspace
            .split_panes_at(screen, fraction)
            .map(|(main, side, divider)| (main.width, side.width, divider.width))
    };
    // A third of 120 columns, the default share.
    assert_eq!(side(1.0), Some((79, 40, 1)));
    assert_eq!(side(0.5), Some((99, 20, 1)));
    assert_eq!(side(0.0), None);
}
