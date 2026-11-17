// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::time::Duration;

use hanafubuki_agent::{SettingChoice, SettingKind};
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::{calls, catalog, center_with, press, screen};

#[test]
fn nested_paths_return_to_each_ancestor_at_wide_and_narrow_widths() {
    for width in [20, 50, 140] {
        let mut catalog = catalog();
        catalog.collections[1].record[0].kind = SettingKind::Choice {
            values: vec![SettingChoice::new("gpt-test", "Test")],
        };
        let mut center = center_with(catalog, Some("modelPolicy"));
        press(&mut center, KeyCode::Enter);
        let shown = screen(&mut center, width, 30);
        if width == 140 {
            assert!(shown.contains("Settings › Model policy › Model"), "{shown}");
        }
        assert_eq!(center.layout.breadcrumbs.len(), 3, "{shown}");
        for ancestor in [1, 0] {
            let (rect, index) = center.layout.breadcrumbs[ancestor];
            assert_eq!(index, ancestor);
            assert!(rect.right() <= width, "{rect:?}");
            center.handle_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: rect.x,
                row: rect.y,
                modifiers: KeyModifiers::NONE,
            });
            assert_eq!(center.pages.len(), ancestor + 1);
            screen(&mut center, width, 30);
        }
        assert!(calls(&mut center).is_empty());
    }
}

#[test]
fn successful_notices_expire_to_blank_but_errors_remain() {
    let mut center = center_with(catalog(), Some("modelPolicy"));
    assert!(!screen(&mut center, 140, 20).contains("Changes save automatically"));
    center.report(Ok("Saved".into()));
    let deadline = center.notice_deadline().unwrap();
    center.expire_notice(deadline - Duration::from_millis(1));
    assert!(screen(&mut center, 140, 20).contains("Saved"));
    center.expire_notice(deadline);
    assert!(!screen(&mut center, 140, 20).contains("Saved"));
    assert!(center.notice_deadline().is_none());
    center.report(Err("Could not save".into()));
    center.expire_notice(deadline + Duration::from_secs(60));
    assert!(screen(&mut center, 140, 20).contains("Could not save"));
    assert!(center.notice_deadline().is_none());
}
