// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The settings center over a catalog made of the configuration's sections
//! (each at its default) and hand-built collections: what it draws at 140
//! and 50 columns, a row for every setting, an untouched center writing
//! nothing, and what each key and click asks of the Agent.

mod interaction;

use std::collections::BTreeMap;

use hanafubuki_agent::{
    config_sections, SettingKind, SettingSource, SettingSpec, SettingState, SettingValue,
    SettingsAction, SettingsActionScope, SettingsCatalog, SettingsCollection, SettingsGroup,
    SettingsRecord,
};
use insta::assert_snapshot;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use super::{CenterRequest, SettingsCenter};
use crate::agent_bridge::{CatalogCall, CatalogOutcome, CatalogReply};
use crate::custom_terminal::Terminal;
use crate::test_support::VT100Backend;

/// Every section of `config.toml` at its default, with one issue.
fn catalog() -> SettingsCatalog {
    let mut sections = config_sections();
    for section in &mut sections {
        section.values = section
            .settings
            .iter()
            .map(|spec| SettingState {
                key: spec.key.clone(),
                value: spec.default.clone(),
                source: SettingSource::Default,
                issue: (spec.key == "memory.maxUnusedDays")
                    .then(|| "the project's value is not a number".to_string()),
            })
            .collect();
    }
    SettingsCatalog {
        sections,
        collections: vec![providers(), policy()],
    }
}

fn text(secret: bool, nullable: bool) -> SettingKind {
    SettingKind::Text {
        maximum_chars: None,
        secret,
        nullable,
    }
}

fn action(id: &str, label: &str, scope: SettingsActionScope, confirm: bool) -> SettingsAction {
    SettingsAction {
        id: id.to_string(),
        label: label.to_string(),
        scope,
        confirm,
        names_record: None,
    }
}

fn providers() -> SettingsCollection {
    let record = |id: &str, label: &str, state: Option<&str>, token: bool| SettingsRecord {
        id: id.to_string(),
        label: label.to_string(),
        summary: format!("responses · https://{id}.example.test/v1"),
        state: state.map(str::to_string),
        issue: None,
        values: BTreeMap::from([
            (
                "displayName".to_string(),
                SettingValue::Text(label.to_string()),
            ),
            ("enabled".to_string(), SettingValue::Boolean(true)),
            (
                "baseUrl".to_string(),
                SettingValue::Text(format!("https://{id}.example.test/v1")),
            ),
        ]),
        secrets_set: if token {
            vec!["apiToken".to_string()]
        } else {
            Vec::new()
        },
    };
    SettingsCollection {
        id: "providers".to_string(),
        title: "Providers".to_string(),
        group: SettingsGroup::Model,
        record: vec![
            SettingSpec::new("displayName", "Name", text(false, false)),
            SettingSpec::new("enabled", "Enabled", SettingKind::Boolean),
            SettingSpec::new("baseUrl", "Base URL", text(false, false)),
            SettingSpec::new("apiToken", "API token", text(true, true)),
        ],
        records: vec![
            record("main", "Main", Some("default"), true),
            record("spare", "Spare", None, false),
        ],
        actions: vec![
            action(
                "add",
                "Add a Provider",
                SettingsActionScope::Collection,
                false,
            ),
            action("save", "Save", SettingsActionScope::Record, false),
            action("delete", "Delete", SettingsActionScope::Record, true),
            action(
                "set_default",
                "Make default",
                SettingsActionScope::Record,
                false,
            ),
        ],
        revision: Some(7),
        issues: Vec::new(),
    }
}

/// One record and no `add`: it opens as its form.
fn policy() -> SettingsCollection {
    SettingsCollection {
        id: "modelPolicy".to_string(),
        title: "Model".to_string(),
        group: SettingsGroup::Model,
        record: vec![SettingSpec::new("model", "Model", text(false, false))],
        records: vec![SettingsRecord {
            id: "policy".to_string(),
            label: "Model policy".to_string(),
            summary: "gpt-test".to_string(),
            state: None,
            issue: None,
            values: BTreeMap::from([(
                "model".to_string(),
                SettingValue::Text("gpt-test".to_string()),
            )]),
            secrets_set: Vec::new(),
        }],
        actions: vec![action("save", "Save", SettingsActionScope::Record, false)],
        revision: Some(7),
        issues: Vec::new(),
    }
}

/// The center once `catalog` is read.
fn center_with(catalog: SettingsCatalog, target: Option<&str>) -> SettingsCenter {
    let mut center = SettingsCenter::new(target.map(str::to_string));
    let read = calls(&mut center);
    assert!(matches!(read.as_slice(), [(_, CatalogCall::Read)]));
    reply(&mut center, read[0].0, Ok(CatalogOutcome::Read), catalog);
    center
}

fn reply(
    center: &mut SettingsCenter,
    request: u64,
    outcome: Result<CatalogOutcome, String>,
    catalog: SettingsCatalog,
) {
    center.on_reply(CatalogReply {
        request,
        outcome,
        catalog: Ok(catalog),
        preferences: None,
    });
}

/// The calls the center asked for since the last look.
fn calls(center: &mut SettingsCenter) -> Vec<(u64, CatalogCall)> {
    center
        .take_requests()
        .into_iter()
        .filter_map(|request| match request {
            CenterRequest::Call(id, call) => Some((id, call)),
            _ => None,
        })
        .collect()
}

fn press(center: &mut SettingsCenter, code: KeyCode) {
    center.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_text(center: &mut SettingsCenter, text: &str) {
    for character in text.chars() {
        press(center, KeyCode::Char(character));
    }
}

/// Right until the tab `label` shows.
fn show_tab(center: &mut SettingsCenter, label: &str) {
    for _ in 0..10 {
        if screen(center, 140, 12).contains(&format!("{label} ")) && selected_tab(center) == label {
            return;
        }
        press(center, KeyCode::Right);
    }
    panic!("no tab {label}");
}

fn selected_tab(center: &SettingsCenter) -> &'static str {
    let catalog = center.catalog.as_ref().unwrap();
    let Some(super::Page::Main(page)) = center.pages.first() else {
        unreachable!();
    };
    super::render::tab_labels(catalog)[page.tab]
}

fn screen(center: &mut SettingsCenter, width: u16, height: u16) -> String {
    let mut terminal = Terminal::with_options(VT100Backend::new(width, height)).unwrap();
    terminal.set_viewport_area(Rect::new(0, 0, width, height));
    {
        let mut frame = terminal.get_frame();
        let area = frame.area();
        center.render(area, frame.buffer_mut());
    }
    terminal.flush().unwrap();
    terminal.backend().to_string()
}

#[test]
fn the_memory_tab_at_140_and_50_columns() {
    let mut center = center_with(catalog(), None);
    show_tab(&mut center, "Memory");
    assert_snapshot!("memory_140", screen(&mut center, 140, 22));
    assert_snapshot!("memory_50", screen(&mut center, 50, 26));
    show_tab(&mut center, "Interface");
    let narrow = screen(&mut center, 50, 26);
    assert!(
        narrow.contains(" Interface "),
        "the selected tab shows:\n{narrow}"
    );
}

#[test]
fn the_interface_tab_lists_its_pages_at_140_columns() {
    let mut center = center_with(catalog(), Some("interface"));
    assert_snapshot!("interface_140", screen(&mut center, 140, 20));
}

#[test]
fn a_collection_and_a_record_at_140_and_50_columns() {
    let mut center = center_with(catalog(), Some("providers"));
    assert_snapshot!("providers_140", screen(&mut center, 140, 14));
    press(&mut center, KeyCode::Enter);
    assert_snapshot!("provider_record_50", screen(&mut center, 50, 24));
}

#[test]
fn a_search_across_the_tabs_at_50_columns() {
    let mut center = center_with(catalog(), None);
    press(&mut center, KeyCode::Char('/'));
    type_text(&mut center, "model");
    assert_snapshot!("search_50", screen(&mut center, 50, 20));
}

/// Every setting of every section is a row of its tab, a conditional one
/// while its condition holds.
#[test]
fn every_setting_has_a_row() {
    let base = catalog();
    for section in &base.sections {
        for spec in &section.settings {
            let mut catalog = base.clone();
            if let Some(condition) = &spec.visible_if {
                for state in catalog
                    .sections
                    .iter_mut()
                    .flat_map(|section| &mut section.values)
                {
                    if state.key == condition.key {
                        state.value = Some(condition.equals.clone());
                    }
                }
            }
            let shown = super::main_page::group_rows(&catalog, section.group)
                .into_iter()
                .any(|row| {
                    matches!(row, super::main_page::MainRow::Setting { section: index, index: setting }
                        if catalog.sections[index].settings[setting].key == spec.key)
                });
            assert!(shown, "{} has no row", spec.key);
        }
    }
}

/// Walking every tab and row, searching and closing writes nothing.
#[test]
fn an_untouched_center_writes_nothing() {
    let mut center = center_with(catalog(), None);
    for _ in 0..10 {
        for _ in 0..40 {
            press(&mut center, KeyCode::Down);
        }
        screen(&mut center, 140, 30);
        press(&mut center, KeyCode::Right);
    }
    press(&mut center, KeyCode::Char('/'));
    type_text(&mut center, "memory");
    press(&mut center, KeyCode::Esc);
    press(&mut center, KeyCode::Esc);
    let requests = center.take_requests();
    assert!(
        matches!(requests.as_slice(), [CenterRequest::Close]),
        "{requests:?}"
    );
}
