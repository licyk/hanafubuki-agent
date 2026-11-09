// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::cell::RefCell;
use std::rc::Rc;

use insta::assert_snapshot;

use super::*;

fn item(id: &str, enabled: bool) -> MultiSelectItem {
    MultiSelectItem {
        id: id.to_string(),
        name: id.to_string(),
        description: Some(format!("the {id}")),
        enabled,
    }
}

fn key(picker: &mut MultiSelectPicker, code: KeyCode) {
    picker.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

fn render(picker: &MultiSelectPicker, width: u16) -> String {
    let height = picker.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    picker.render(area, &mut buf);
    (0..height)
        .map(|row| {
            (0..width)
                .map(|column| buf[(column, row)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The picker over `a` (on), `b` and `c`, its preview the ids on, and
/// what it confirmed.
fn picker() -> (MultiSelectPicker, Rc<RefCell<Option<Vec<String>>>>) {
    let confirmed = Rc::new(RefCell::new(None));
    let sink = confirmed.clone();
    let picker = MultiSelectPicker::new(
        "Configure".to_string(),
        Some("Pick items.".to_string()),
        vec![item("a", true), item("b", false), item("c", false)],
    )
    .on_preview(Box::new(|items| {
        let on: Vec<&str> = items
            .iter()
            .filter(|item| item.enabled)
            .map(|item| item.id.as_str())
            .collect();
        Some(Line::from(on.join(" | ")))
    }))
    .on_confirm(Box::new(move |ids| *sink.borrow_mut() = Some(ids.to_vec())));
    (picker, confirmed)
}

#[test]
fn space_toggles_arrows_reorder_and_enter_confirms_the_order() {
    let (mut picker, confirmed) = picker();
    key(&mut picker, KeyCode::Down);
    key(&mut picker, KeyCode::Char(' '));
    // `b` moves above `a`, the cursor with it.
    key(&mut picker, KeyCode::Left);
    key(&mut picker, KeyCode::Left);
    assert_eq!(picker.preview_line, Some(Line::from("b | a")));
    key(&mut picker, KeyCode::Down);
    key(&mut picker, KeyCode::Down);
    key(&mut picker, KeyCode::Char(' '));
    key(&mut picker, KeyCode::Right);
    key(&mut picker, KeyCode::Enter);
    assert!(picker.is_complete());
    assert_eq!(
        confirmed.borrow().as_deref(),
        Some(&["b".to_string(), "a".to_string(), "c".to_string()][..])
    );
}

#[test]
fn escape_cancels_without_confirming() {
    let cancelled = Rc::new(RefCell::new(false));
    let sink = cancelled.clone();
    let (picker, confirmed) = picker();
    let mut picker = picker.on_cancel(Box::new(move || *sink.borrow_mut() = true));
    key(&mut picker, KeyCode::Char(' '));
    key(&mut picker, KeyCode::Esc);
    assert!(picker.is_complete());
    assert!(*cancelled.borrow());
    assert_eq!(*confirmed.borrow(), None);
}

#[test]
fn the_picker_shows_its_items_preview_and_keys() {
    let (mut picker, _) = picker();
    key(&mut picker, KeyCode::Down);
    key(&mut picker, KeyCode::Char(' '));
    assert_snapshot!(render(&picker, 60));
}
