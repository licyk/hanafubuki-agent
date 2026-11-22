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
