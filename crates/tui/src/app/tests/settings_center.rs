//! `/config`'s settings center end to end over the standalone Agent, in
//! both presentations: a toggle saved at once and taken by the interface,
//! a search across the tabs, an edit refused with its reason then saved, a
//! page that is a command's views returning to its row, a write a profile
//! overrides, and the pointer.

use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::Harness;
use crate::app_event::AppEvent;
use crate::history_cell::HistoryRenderMode;
use crate::transcript_mode::TranscriptMode;

impl Harness {
    /// Down until the selected row is `label`.
    fn select_setting(&mut self, label: &str) {
        let selected = format!("› {label}");
        for _ in 0..80 {
            self.app.draw(&mut self.tui).unwrap();
            if self.screen().contains(&selected) {
                return;
            }
            self.key(KeyCode::Down);
        }
        panic!("no row {label}:\n{}", self.screen());
    }

    /// A left press on the cell at `column` and `row`.
    fn press_at(&mut self, column: u16, row: u16) {
        let event = AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
        assert!(self.app.handle_event(&mut self.tui, event));
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
            !harness.screen().contains("Changes save automatically")
                && !harness.screen().contains("Saved")
        });
    }
}

#[test]
fn a_search_finds_a_setting_of_another_tab_and_an_edit_says_why_it_is_refused() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/config");
    harness.pump_until_screen_shows("Changes save automatically");
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
    assert!(harness.screen().contains("Settings › theme"));
    harness.key(KeyCode::Esc);
    harness.pump_until("the tab again", |harness| {
        harness.screen().contains("› Syntax theme") && !harness.screen().contains("Select Syntax")
    });

    // A page of its own row, and Esc from it.
    harness.select_setting("Keymap");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Pick an action");
    harness.key(KeyCode::Esc);
    harness.pump_until("the tab again", |harness| {
        harness.screen().contains("› Keymap")
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
    harness.pump_until_screen_shows("Changes save automatically");
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
