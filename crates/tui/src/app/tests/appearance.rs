//! `/theme`, `/statusline` and `/title` end to end over the standalone
//! Agent: what a command's arguments change for the run, what a setup view
//! saves into `config.toml`, and the title the app writes.

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::KeyCode;

use super::Harness;

#[test]
fn statusline_arguments_apply_to_the_run_and_the_setup_saves() {
    let mut harness = Harness::owned(vec![assistant_events("The answer is 42.")]);
    harness.pump_until_screen_shows("fixture-model · /project");

    harness.type_and_submit("/statusline model --no-colors");
    harness.pump_until_screen_shows("Updated for this session; it is not saved.");
    let bottom = harness.bottom_rows(1);
    assert_eq!(bottom.trim(), "fixture-model", "{bottom}");
    assert!(!harness.config_file().contains("statusLine"));

    harness.type_and_submit("/statusline nonsense");
    harness.pump_until_screen_shows("Unknown status-line item 'nonsense'");

    // The setup lists the items shown first, then the others in their
    // order: `model` is turned off and `current-dir`, the fourth, on.
    harness.type_and_submit("/statusline");
    harness.pump_until_screen_shows("Configure Status Line");
    harness.key(KeyCode::Char(' '));
    for _ in 0..3 {
        harness.key(KeyCode::Down);
    }
    harness.key(KeyCode::Char(' '));
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Status line updated.");
    let config = harness.config_file();
    assert!(
        config.contains("statusLine = [\"current-dir\"]"),
        "{config}"
    );
    assert!(config.contains("statusLineColors = false"), "{config}");
    assert_eq!(harness.bottom_rows(1).trim(), "/project");
}

#[test]
fn the_status_line_follows_the_session() {
    let mut harness = Harness::owned(vec![assistant_events("The answer is 42.")]);
    harness.pump_until_screen_shows("fixture-model");
    harness.type_and_submit("/statusline session-id used-tokens");
    harness.type_and_submit("hello");
    harness.pump_until_turn_ends();
    harness.pump_until("the status line to show the session", |harness| {
        harness.bottom_rows(1).contains("used")
    });
    let session_id = harness.app.bridge.session_id().unwrap().to_string();
    let bottom = harness.bottom_rows(1);
    assert!(bottom.contains(&session_id), "{bottom}");
}

#[test]
fn title_sets_the_terminal_title_for_the_run_and_clear_gives_it_back() {
    let mut harness = Harness::owned(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    // The default title has nothing to show before the session has a
    // title or a turn runs, but the project's name.
    assert_eq!(harness.app.terminal_title.written(), Some("project"));

    harness.type_and_submit("/title app-name model");
    harness.pump_until_screen_shows("Updated for this session; it is not saved.");
    assert_eq!(
        harness.app.terminal_title.written(),
        Some("hanafubuki-agent | fixture-model")
    );
    harness.type_and_submit("/title --clear");
    harness.pump_until("the title to be given back", |harness| {
        harness.app.terminal_title.written().is_none()
    });
    assert!(!harness.config_file().contains("terminalTitle"));

    harness.type_and_submit("/title");
    harness.pump_until_screen_shows("Configure Terminal Title");
    harness.key(KeyCode::Char(' '));
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Terminal title updated.");
    assert!(harness
        .config_file()
        .contains("terminalTitle = [\"app-name\"]"));
    assert_eq!(
        harness.app.terminal_title.written(),
        Some("hanafubuki-agent")
    );
}

#[test]
fn theme_saves_the_theme_picked() {
    let mut harness = Harness::owned(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    harness.type_and_submit("/theme");
    harness.pump_until_screen_shows("Select Syntax Theme");
    for character in "nord".chars() {
        harness.key(KeyCode::Char(character));
    }
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Theme set to nord.");
    assert!(harness.config_file().contains("theme = \"nord\""));
}
