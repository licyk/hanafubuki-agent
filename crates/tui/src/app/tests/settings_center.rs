//! `/config`'s settings center end to end over the standalone Agent, in
//! both presentations: a toggle saved at once and taken by the interface,
//! a search across the tabs, an edit refused with its reason then saved, a
//! page that is a command's views returning to its row, a write a profile
//! overrides, and the pointer.

use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use hanafubuki_agent::test_support::FakeListing;

use super::Harness;
use crate::app_event::AppEvent;
use crate::history_cell::HistoryRenderMode;
use crate::transcript_mode::TranscriptMode;

/// A models endpoint's answer listing `ids`.
fn the_models(ids: &[&str]) -> Vec<u8> {
    let data: Vec<_> = ids
        .iter()
        .map(|id| serde_json::json!({ "id": id }))
        .collect();
    serde_json::json!({ "data": data }).to_string().into_bytes()
}

impl Harness {
    /// A left press on the cell at `column` and `row`.
    pub(super) fn press_at(&mut self, column: u16, row: u16) {
        let event = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
        assert!(self.app.handle_event(&mut self.tui, event));
    }
}

/// The Model overrides' `a` is the same list `/model` is, over the
/// Providers: the model chosen is written as its own settings, and the
/// center comes back to the collection with it selected.
#[test]
fn a_model_override_is_added_from_the_list_of_the_providers_models() {
    let mut harness = Harness::listing(
        vec![FakeListing::ok(
            &String::from_utf8(the_models(&["first-model"])).unwrap(),
        )],
        TranscriptMode::Owned,
        |_| {},
    );
    harness.type_and_submit("/config providerModels");
    harness.pump_until_screen_shows("+ Add a model");
    harness.key(KeyCode::Char('a'));
    let screen = harness.pump_until_screen_shows("Add model overrides");
    assert!(
        screen.contains("Settings › Model overrides › Add a model"),
        "{screen}"
    );
    harness.pump_until_screen_shows("first-model");
    // The session's model is pinned first; the listing's follow it.
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    let screen = harness.pump_until_screen_shows("Added");
    assert!(
        screen.contains("Fixture Provider · first-model"),
        "{screen}"
    );
    let registry = harness.registry();
    let provider = registry
        .providers
        .iter()
        .find(|provider| provider.id == "provider-1")
        .unwrap();
    assert!(
        provider.models.contains_key("first-model"),
        "{:?}",
        provider.models
    );
}

#[test]
fn a_breadcrumb_leaves_the_hosted_model_picker_without_saving() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        harness.type_and_submit("/providers policy");
        harness.pump_until_screen_shows("Settings › Model policy");
        harness.select_setting("Model");
        let screen = harness.screen();
        let row = screen
            .lines()
            .position(|line| line.trim_start().starts_with("› Model "))
            .unwrap();
        harness.press_at(6, row as u16);
        harness.pump_until_screen_shows("Settings › Model policy › Model");
        assert!(harness.app.chat_widget.has_command_view("providers"));
        let previous = harness.registry().model_policy.model.clone();
        let (row, column) = harness.find("Model policy");
        harness.press_at(column, row);
        harness.pump_until("the model policy form", |harness| {
            !harness.app.chat_widget.has_command_view("providers")
                && harness.screen().contains("Reasoning effort")
        });
        assert_eq!(harness.registry().model_policy.model, previous);
        let (row, column) = harness.find("Settings");
        harness.press_at(column, row);
        harness.pump_until_screen_shows("esc close");
        assert!(!harness.screen().contains("Settings ›"));
    }
}

#[test]
fn a_toggle_saves_at_once_in_both_presentations() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        harness.type_and_submit("/config interface");
        harness.pump_until_screen_shows("Terminal interface");
        harness.select_setting("Raw output");
        harness.key(KeyCode::Enter);
        harness.pump_until_screen_shows("Saved");
        assert!(harness.config_file().contains("rawOutput = true"));
        // The interface takes it at once.
        assert_eq!(harness.app.render_mode, HistoryRenderMode::Raw);
        harness.key(KeyCode::Esc);
        harness.pump_until("the center to close", |harness| {
            harness.app.workspace.settings().is_none()
        });
    }
}

#[test]
fn a_search_finds_a_setting_of_another_tab_and_an_edit_says_why_it_is_refused() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/config");
    harness.pump_until_screen_shows("esc close");
    harness.key(KeyCode::Char('/'));
    for character in "source age".chars() {
        harness.key(KeyCode::Char(character));
    }
    harness.pump_until_screen_shows("Memory · Memory");
    harness.key(KeyCode::Enter);
    harness.key(KeyCode::Enter);
    for _ in 0..4 {
        harness.key(KeyCode::Backspace);
    }
    harness.key(KeyCode::Char('0'));
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("is between 1 and 3650");
    harness.key(KeyCode::Backspace);
    harness.key(KeyCode::Char('4'));
    harness.key(KeyCode::Char('5'));
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved");
    let file = harness.config_file();
    assert!(file.contains("maxSourceAgeDays = 45"), "{file}");
}

#[test]
fn esc_from_a_page_returns_to_the_row_that_opened_it() {
    let mut harness = Harness::owned(Vec::new());
    harness.type_and_submit("/config interface");
    harness.pump_until_screen_shows("Terminal interface");
    harness.select_setting("Syntax theme");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Select Syntax Theme");
    assert!(harness.screen().contains("Settings › Syntax theme"));
    harness.key(KeyCode::Esc);
    harness.pump_until("the tab again", |harness| {
        harness
            .app
            .workspace
            .settings()
            .is_some_and(|center| center.hosted_command().is_none())
            && harness.screen().contains("› Syntax theme")
    });

    // A page of its own row, and Esc from it.
    harness.select_setting("Keymap");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Pick an action");
    harness.key(KeyCode::Esc);
    harness.pump_until("the tab again", |harness| {
        harness
            .app
            .workspace
            .settings()
            .is_some_and(|center| center.hosted_command().is_none())
            && harness.screen().contains("› Keymap")
    });
}

#[test]
fn a_write_a_profile_overrides_says_so() {
    let mut harness = Harness::start_layered(
        Vec::new(),
        true,
        TranscriptMode::Terminal,
        |config| {
            config
                .set_user_json("profiles.work.memory.useMemories", false.into())
                .unwrap()
        },
        Some("work"),
    );
    harness.type_and_submit("/config memory");
    harness.pump_until_screen_shows("Use memories");
    harness.select_setting("Use memories");
    assert!(
        harness.screen().contains("· profile work"),
        "{}",
        harness.screen()
    );
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved · the profile work layer overrides it");
    assert!(harness.config_file().contains("useMemories = true"));
}

#[test]
fn a_click_selects_a_tab_and_a_row_and_a_second_click_toggles_it() {
    let mut harness = Harness::owned(Vec::new());
    harness.type_and_submit("/config");
    harness.pump_until_screen_shows("esc close");
    let (row, column) = harness.find(" Memory ");
    harness.press_at(column + 2, row);
    harness.pump_until_screen_shows("Generate memories");
    let (row, _) = harness.find("Generate memories");
    harness.press_at(6, row);
    harness.pump_until_screen_shows("› Generate memories");
    harness.press_at(6, row);
    harness.pump_until_screen_shows("Saved");
    assert!(harness.config_file().contains("generateMemories = false"));
}
