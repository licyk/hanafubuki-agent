// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What the center's keys and clicks ask of the Agent: a toggle written at
//! once, an edit checked first and kept when refused, a record's field
//! saved at the collection's revision, a confirmation before a delete, the
//! search opening a setting of another tab, and the pointer.

use hanafubuki_agent::{SettingWriteOp, SettingsActionOutcome, SettingsWriteOutcome};
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use serde_json::json;

use super::{calls, catalog, center_with, press, reply, screen, show_tab, type_text};
use crate::agent_bridge::{CatalogCall, CatalogOutcome};
use crate::settings_center::SettingsCenter;

fn written() -> CatalogOutcome {
    CatalogOutcome::Written(SettingsWriteOutcome {
        written: Vec::new(),
        shadowed: Vec::new(),
        apply: Vec::new(),
    })
}

/// Down until the selected row shows `label`.
fn select(center: &mut SettingsCenter, label: &str) {
    for _ in 0..60 {
        let screen = screen(center, 140, 40);
        if screen
            .lines()
            .any(|line| line.starts_with(&format!("  › {label}")))
        {
            return;
        }
        press(center, KeyCode::Down);
    }
    panic!("no row {label}:\n{}", screen(center, 140, 40));
}

#[test]
fn a_boolean_is_written_at_once() {
    let mut center = center_with(catalog(), None);
    show_tab(&mut center, "Memory");
    select(&mut center, "Memory");
    press(&mut center, KeyCode::Enter);
    let sent = calls(&mut center);
    let [(_, CatalogCall::Write(write))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(write.changes[0].key, "memory.enabled");
    assert_eq!(write.changes[0].op, SettingWriteOp::Set(json!(true)));
    assert!(screen(&mut center, 140, 10).contains("Saving…"));
    reply(&mut center, sent[0].0, Ok(written()), catalog());
    assert!(screen(&mut center, 140, 10).contains("Saved"));
}

#[test]
fn an_edit_is_checked_first_and_kept_when_refused() {
    let mut center = center_with(catalog(), None);
    show_tab(&mut center, "Memory");
    select(&mut center, "Maximum source age");
    press(&mut center, KeyCode::Enter);
    for _ in 0..4 {
        press(&mut center, KeyCode::Backspace);
    }
    type_text(&mut center, "99999");
    press(&mut center, KeyCode::Enter);
    assert!(calls(&mut center).is_empty());
    let shown = screen(&mut center, 140, 40);
    assert!(shown.contains("is between 1 and 3650"), "{shown}");

    for _ in 0..5 {
        press(&mut center, KeyCode::Backspace);
    }
    type_text(&mut center, "30");
    press(&mut center, KeyCode::Enter);
    let sent = calls(&mut center);
    let [(id, CatalogCall::Write(write))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(write.changes[0].op, SettingWriteOp::Set(json!(30)));
    reply(
        &mut center,
        *id,
        Err("the file is read-only".to_string()),
        catalog(),
    );
    let shown = screen(&mut center, 140, 40);
    assert!(shown.contains("✗ the file is read-only"), "{shown}");
    // The editor stays, with what was typed.
    assert!(shown.contains("› Maximum source age"), "{shown}");
    assert!(shown.contains(" 30 "), "{shown}");
}

#[test]
fn reset_removes_what_the_user_set() {
    let mut catalog = catalog();
    let state = catalog
        .sections
        .iter_mut()
        .flat_map(|section| &mut section.values)
        .find(|state| state.key == "memory.enabled")
        .unwrap();
    state.source = hanafubuki_agent::SettingSource::User;
    let mut center = center_with(catalog, Some("memory"));
    select(&mut center, "Memory");
    assert!(screen(&mut center, 140, 40).contains("enter toggle · r reset"));
    press(&mut center, KeyCode::Char('r'));
    let sent = calls(&mut center);
    let [(_, CatalogCall::Write(write))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(write.changes[0].op, SettingWriteOp::Reset);
}

#[test]
fn a_record_field_is_saved_at_the_revision_and_a_refused_one_goes_with_the_next() {
    let mut center = center_with(catalog(), Some("providers"));
    press(&mut center, KeyCode::Enter);
    let shown = screen(&mut center, 140, 20);
    assert!(shown.contains("Providers › Main"), "{shown}");
    assert!(shown.contains("••••"), "{shown}");
    select(&mut center, "Enabled");
    press(&mut center, KeyCode::Enter);
    let sent = calls(&mut center);
    let [(id, CatalogCall::Action(request))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(request.action, "save");
    assert_eq!(request.record.as_deref(), Some("main"));
    assert_eq!(request.expected_revision, Some(7));
    assert_eq!(
        request.values,
        json!({ "enabled": false }).as_object().unwrap().clone()
    );
    reply(
        &mut center,
        *id,
        Err("the revision moved".to_string()),
        catalog(),
    );
    assert!(screen(&mut center, 140, 20).contains("unsaved"));

    select(&mut center, "Base URL");
    press(&mut center, KeyCode::Enter);
    type_text(&mut center, "2");
    press(&mut center, KeyCode::Enter);
    let sent = calls(&mut center);
    let [(_, CatalogCall::Action(request))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(
        request.values,
        json!({ "enabled": false, "baseUrl": "https://main.example.test/v12" })
            .as_object()
            .unwrap()
            .clone()
    );
}

#[test]
fn a_delete_is_confirmed_first() {
    let mut center = center_with(catalog(), Some("providers"));
    press(&mut center, KeyCode::Down);
    press(&mut center, KeyCode::Char('d'));
    let shown = screen(&mut center, 140, 20);
    assert!(shown.contains("Delete?"), "{shown}");
    assert!(shown.contains("Spare"), "{shown}");
    press(&mut center, KeyCode::Enter);
    assert!(calls(&mut center).is_empty(), "Cancel is selected first");
    press(&mut center, KeyCode::Char('d'));
    press(&mut center, KeyCode::Down);
    press(&mut center, KeyCode::Enter);
    let sent = calls(&mut center);
    let [(id, CatalogCall::Action(request))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(request.action, "delete");
    assert_eq!(request.record.as_deref(), Some("spare"));
    let mut after = catalog();
    after.collections[0].records.pop();
    reply(
        &mut center,
        *id,
        Ok(CatalogOutcome::Acted(SettingsActionOutcome::default())),
        after,
    );
    let shown = screen(&mut center, 140, 20);
    assert!(!shown.contains("Spare"), "{shown}");
}

#[test]
fn an_add_names_the_record_when_its_owner_does_not() {
    let mut catalog = catalog();
    catalog.collections[0].actions[0].names_record = Some("Provider id".to_string());
    let mut center = center_with(catalog, Some("providers"));
    press(&mut center, KeyCode::Char('a'));
    press(&mut center, KeyCode::Enter);
    type_text(&mut center, "third");
    press(&mut center, KeyCode::Enter);
    select(&mut center, "Name");
    press(&mut center, KeyCode::Enter);
    type_text(&mut center, "Third");
    press(&mut center, KeyCode::Enter);
    assert!(calls(&mut center).is_empty(), "a draft is saved by its Add");
    select(&mut center, "Add a Provider");
    press(&mut center, KeyCode::Enter);
    let sent = calls(&mut center);
    let [(_, CatalogCall::Action(request))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(request.action, "add");
    assert_eq!(request.record.as_deref(), Some("third"));
    assert_eq!(
        request.values,
        json!({ "displayName": "Third" })
            .as_object()
            .unwrap()
            .clone()
    );
}

#[test]
fn a_search_opens_a_choice_of_another_tab() {
    let mut center = center_with(catalog(), None);
    press(&mut center, KeyCode::Char('/'));
    type_text(&mut center, "strategy");
    press(&mut center, KeyCode::Enter);
    let shown = screen(&mut center, 140, 20);
    assert!(shown.contains("Context · Compaction"), "{shown}");
    press(&mut center, KeyCode::Enter);
    let shown = screen(&mut center, 140, 20);
    assert!(shown.contains("(current)"), "{shown}");
    press(&mut center, KeyCode::Char('2'));
    let sent = calls(&mut center);
    let [(_, CatalogCall::Write(write))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(write.changes[0].key, "compaction.strategy");
}

#[test]
fn a_click_selects_a_tab_and_a_row_and_a_second_click_opens_it() {
    let mut center = center_with(catalog(), None);
    let shown = screen(&mut center, 140, 30);
    let (row, line) = shown
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(" Memory "))
        .unwrap();
    let column = line.find(" Memory ").unwrap() as u16 + 2;
    let click = |column: u16, row: u16| MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    center.handle_mouse(click(column, row as u16));
    let shown = screen(&mut center, 140, 30);
    let (row, _) = shown
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("Use memories"))
        .unwrap();
    center.handle_mouse(click(10, row as u16));
    assert!(screen(&mut center, 140, 30).contains("› Use memories"));
    assert!(calls(&mut center).is_empty());
    center.handle_mouse(click(10, row as u16));
    let sent = calls(&mut center);
    let [(_, CatalogCall::Write(write))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(write.changes[0].key, "memory.useMemories");
}

#[test]
fn a_commit_group_is_a_form_saved_together() {
    let mut center = center_with(catalog(), Some("web"));
    select(&mut center, "Web search provider");
    press(&mut center, KeyCode::Enter);
    let shown = screen(&mut center, 140, 20);
    assert!(shown.contains("Settings › Web tools"), "{shown}");
    assert!(shown.contains("saved together"), "{shown}");
    press(&mut center, KeyCode::Enter);
    press(&mut center, KeyCode::Char('3'));
    select(&mut center, "SearXNG URL");
    press(&mut center, KeyCode::Enter);
    type_text(&mut center, "https://search.example.test");
    press(&mut center, KeyCode::Enter);
    assert!(calls(&mut center).is_empty(), "a draft until Save");
    select(&mut center, "Save");
    press(&mut center, KeyCode::Enter);
    let sent = calls(&mut center);
    let [(id, CatalogCall::Write(write))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    let keys = write
        .changes
        .iter()
        .map(|change| change.key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, ["searxngBaseUrl", "webSearchProvider"]);
    reply(&mut center, *id, Ok(written()), catalog());
    assert!(!screen(&mut center, 140, 20).contains("Web tools ›"));
}
