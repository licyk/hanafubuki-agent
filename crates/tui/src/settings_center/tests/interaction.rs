// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What the center's keys and clicks ask of the Agent: a toggle written at
//! once, an edit checked first and kept when refused, a record's field
//! saved at the collection's revision, a confirmation before a delete, an
//! add starting from its defaults, a record shown only, the search opening
//! a setting of another tab, and the pointer.

use hanafubuki_agent::{
    SettingChoice, SettingCondition, SettingKind, SettingSpec, SettingValue, SettingWriteOp,
    SettingsActionOutcome, SettingsCatalog, SettingsWriteOutcome,
};
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
fn the_busy_notice_names_the_action_that_runs_and_an_edit_is_saving() {
    let mut center = center_with(catalog(), Some("providers"));
    press(&mut center, KeyCode::Enter);
    // A record's field is an edit, though the record's save writes it.
    select(&mut center, "Enabled");
    press(&mut center, KeyCode::Enter);
    let sent = calls(&mut center);
    let shown = screen(&mut center, 140, 20);
    assert!(shown.contains("Saving…"), "{shown}");
    let saved = Ok(CatalogOutcome::Acted(SettingsActionOutcome::default()));
    reply(&mut center, sent[0].0, saved, catalog());

    select(&mut center, "Make default");
    press(&mut center, KeyCode::Enter);
    let sent = calls(&mut center);
    let [(id, CatalogCall::Action(request))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(request.action, "set_default");
    let shown = screen(&mut center, 140, 20);
    assert!(
        shown.contains("Make default…") && !shown.contains("Saving…"),
        "{shown}"
    );
    let acted = SettingsActionOutcome {
        record: None,
        message: Some("Main is the default".to_string()),
    };
    reply(
        &mut center,
        *id,
        Ok(CatalogOutcome::Acted(acted)),
        catalog(),
    );
    let shown = screen(&mut center, 140, 20);
    assert!(
        shown.contains("Main is the default") && !shown.contains("Make default…"),
        "{shown}"
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

#[test]
fn an_add_starts_from_its_defaults() {
    let mut catalog = catalog();
    let record = &mut catalog.collections[0].record;
    for spec in record.iter_mut() {
        if spec.key == "enabled" {
            spec.default = Some(SettingValue::Boolean(true));
        }
    }
    record.push(SettingSpec {
        default: Some(SettingValue::Text("stdio".into())),
        ..SettingSpec::new(
            "transport.type",
            "Transport",
            SettingKind::Choice {
                values: vec![
                    SettingChoice::new("stdio", "Standard input and output"),
                    SettingChoice::new("http", "HTTP"),
                ],
            },
        )
    });
    record.push(SettingSpec {
        visible_if: Some(SettingCondition {
            key: "transport.type".into(),
            equals: SettingValue::Text("stdio".into()),
        }),
        ..SettingSpec::new(
            "transport.program",
            "Program",
            SettingKind::Text {
                maximum_chars: None,
                secret: false,
                nullable: false,
            },
        )
    });
    let mut center = center_with(catalog, Some("providers"));
    press(&mut center, KeyCode::Char('a'));
    let shown = screen(&mut center, 140, 20);
    assert!(shown.contains("[x] On"), "{shown}");
    assert!(shown.contains("Standard input and output"), "{shown}");
    assert!(
        shown.contains("Program"),
        "its transport's fields show: {shown}"
    );
    select(&mut center, "Enabled");
    press(&mut center, KeyCode::Enter);
    assert!(screen(&mut center, 140, 20).contains("[ ] Off"));
    select(&mut center, "Add a Provider");
    press(&mut center, KeyCode::Enter);
    let sent = calls(&mut center);
    let [(_, CatalogCall::Action(request))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(
        request.values,
        json!({ "enabled": false }).as_object().unwrap().clone(),
        "only what was changed is sent; the rest is what the add writes"
    );
}

#[test]
fn a_record_written_elsewhere_is_shown_with_why() {
    let mut catalog = catalog();
    catalog.collections[0].records[0].read_only = Some("Written in its role file".to_string());
    let mut center = center_with(catalog, Some("providers"));
    press(&mut center, KeyCode::Enter);
    let shown = screen(&mut center, 140, 20);
    assert!(shown.contains("Written in its role file"), "{shown}");
    select(&mut center, "Enabled");
    press(&mut center, KeyCode::Enter);
    press(&mut center, KeyCode::Char('r'));
    assert!(calls(&mut center).is_empty());
}

/// The center at the policy's effort, which offers `levels` and is
/// `effort`.
fn with_effort(levels: Vec<SettingChoice>, effort: Option<&str>) -> SettingsCenter {
    let mut center = center_with(effort_catalog(levels, effort), Some("modelPolicy"));
    select(&mut center, "Reasoning effort");
    center
}

fn effort_catalog(levels: Vec<SettingChoice>, effort: Option<&str>) -> SettingsCatalog {
    let mut catalog = catalog();
    let policy = catalog
        .collections
        .iter_mut()
        .find(|collection| collection.id == "modelPolicy")
        .unwrap();
    policy.record.push(SettingSpec::new(
        "reasoningEffort",
        "Reasoning effort",
        SettingKind::ChoiceOrText {
            values: levels,
            maximum_chars: Some(64),
            nullable: true,
        },
    ));
    if let Some(effort) = effort {
        policy.records[0].values.insert(
            "reasoningEffort".to_string(),
            SettingValue::Text(effort.to_string()),
        );
    }
    catalog
}

fn saved_values(center: &mut SettingsCenter) -> (u64, serde_json::Value) {
    let sent = calls(center);
    let [(id, CatalogCall::Action(request))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(request.action, "save");
    (*id, serde_json::Value::Object(request.values.clone()))
}

#[test]
fn an_effort_offers_what_effort_offers_and_a_level_is_saved() {
    let levels = vec![
        SettingChoice::new("low", "low"),
        SettingChoice::new("high", "high (model default)"),
    ];
    let mut center = with_effort(levels.clone(), Some("max"));
    let shown = screen(&mut center, 140, 20);
    assert!(shown.contains("Reasoning effort  max ›"), "{shown}");
    press(&mut center, KeyCode::Enter);
    let shown = screen(&mut center, 140, 20);
    let offered = crate::model_facts::effort_options(&levels, Some("max"));
    let mut lines = shown.lines();
    for (index, option) in offered.iter().enumerate() {
        let current = if option.current { " (current)" } else { "" };
        let row = format!("{}. {}{current}", index + 1, option.label);
        assert!(lines.any(|line| line.contains(&row)), "{row}:\n{shown}");
    }
    assert!(shown.contains("4. Custom: max (current)"), "{shown}");
    press(&mut center, KeyCode::Char('2'));
    assert_eq!(
        saved_values(&mut center).1,
        json!({ "reasoningEffort": "high" })
    );
}

#[test]
fn with_no_levels_an_effort_offers_default_and_custom_and_default_unsets_it() {
    let mut center = with_effort(Vec::new(), Some("high"));
    press(&mut center, KeyCode::Enter);
    let shown = screen(&mut center, 140, 20);
    assert!(shown.contains("1. Default"), "{shown}");
    assert!(shown.contains("2. Custom: high (current)"), "{shown}");
    assert!(!shown.contains("3. "), "{shown}");
    press(&mut center, KeyCode::Char('1'));
    assert_eq!(
        saved_values(&mut center).1,
        json!({ "reasoningEffort": null })
    );
}

#[test]
fn a_custom_effort_is_typed_in_its_row_and_a_refused_one_stays_with_why() {
    let levels = vec![SettingChoice::new("low", "low")];
    let mut center = with_effort(levels.clone(), None);
    press(&mut center, KeyCode::Enter);
    press(&mut center, KeyCode::Char('3'));
    assert!(
        calls(&mut center).is_empty(),
        "Custom opens the row's editor"
    );
    type_text(&mut center, "none");
    press(&mut center, KeyCode::Enter);
    let (id, values) = saved_values(&mut center);
    assert_eq!(values, json!({ "reasoningEffort": "none" }));
    reply(
        &mut center,
        id,
        Err("reasoning effort 'none' is not supported".to_string()),
        effort_catalog(levels, None),
    );
    let shown = screen(&mut center, 140, 20);
    assert!(
        shown.contains("reasoning effort 'none' is not supported"),
        "{shown}"
    );
    for _ in 0.."none".len() {
        press(&mut center, KeyCode::Backspace);
    }
    type_text(&mut center, "max");
    press(&mut center, KeyCode::Enter);
    assert_eq!(
        saved_values(&mut center).1,
        json!({ "reasoningEffort": "max" })
    );
}

#[test]
fn reset_unsets_a_records_field() {
    let mut center = center_with(catalog(), Some("providers"));
    press(&mut center, KeyCode::Enter);
    select(&mut center, "Base URL");
    press(&mut center, KeyCode::Char('r'));
    let sent = calls(&mut center);
    let [(_, CatalogCall::Action(request))] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(request.action, "save");
    assert_eq!(
        request.values,
        json!({ "baseUrl": null }).as_object().unwrap().clone()
    );
}
