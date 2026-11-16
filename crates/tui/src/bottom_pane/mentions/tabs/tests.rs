// Modified from OpenAI Codex 57a808a392 (Apache-2.0).
use super::*;
#[test]
fn narrow_tabs_keep_active_label_and_navigation() {
    let area = Rect::new(0, 0, 18, 1);
    let mut buffer = Buffer::empty(area);
    let regions = render_filled_tab_bar(
        &["All Results", "Filesystem Only", "Plugins"],
        2,
        area,
        &mut buffer,
    );
    assert!(regions.iter().any(|(index, _)| *index == 2));
    let text = (0..18).map(|x| buffer[(x, 0)].symbol()).collect::<String>();
    assert!(text.contains("Plugins"));
    assert!(text.contains('‹'));
}
#[test]
fn tiny_tabs_and_empty_labels_do_not_overflow() {
    for width in 0..8 {
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        render_filled_tab_bar(&["Files", "Plugins"], 1, area, &mut buffer);
        assert!(render_filled_tab_bar(&[], 0, area, &mut buffer).is_empty());
    }
}
