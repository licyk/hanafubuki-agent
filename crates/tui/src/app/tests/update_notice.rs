// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The update notice end to end: from the command line's cache at launch
//! it is in the first frame; from its check mid-session (the notifier) it
//! appears inside the session header, in both
//! presentations; a settings write keeps it, and both conversations of a
//! side conversation show it.

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::AgentTuiPreferences;
use ratatui::crossterm::event::KeyCode;

use super::Harness;
use crate::appearance::AppearanceEvent;
use crate::transcript_mode::TranscriptMode;
use crate::update_notice::{TuiUpdateNotice, TuiUpdateNotifier};

const FULL: &str = "Update available: 0.3.0 · hanafubuki-agent update";
const SHORT: &str = "Update available: 0.3.0";
/// The default status line before the first message.
const STATUS: &str = "fixture-model · /project";

fn notice(version: &str) -> TuiUpdateNotice {
    TuiUpdateNotice {
        version: version.to_string(),
        action: "hanafubuki-agent update".to_string(),
    }
}

impl Harness {
    fn notifier(&self) -> TuiUpdateNotifier {
        TuiUpdateNotifier::new(self.app.app_event_tx.clone())
    }

    /// The screen's row that holds `text`.
    fn row_of(&self, text: &str) -> Option<usize> {
        self.screen().lines().position(|line| line.contains(text))
    }

    /// The composer's prompt row.
    fn composer_row(&self) -> usize {
        self.screen()
            .lines()
            .position(|line| line.starts_with('›'))
            .unwrap_or_else(|| panic!("no composer:\n{}", self.screen()))
    }
}

#[test]
fn a_notice_from_the_check_updates_the_header_without_occupying_the_footer() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        harness.pump_until_screen_shows("directory:");
        assert_eq!(harness.row_of("Update available"), None);

        harness.notifier().notify(notice("0.3.0"));
        harness.pump_until_screen_shows(FULL);

        let screen = harness.screen();
        let update_row = harness.row_of(FULL).unwrap();
        assert!(
            update_row > harness.row_of("directory:").unwrap(),
            "{screen}"
        );
        assert!(update_row < harness.row_of("╰").unwrap(), "{screen}");
        assert!(update_row < harness.composer_row(), "{screen}");
        assert_ne!(Some(update_row), harness.row_of(STATUS), "{screen}");
        assert_eq!(screen.matches(SHORT).count(), 1, "{screen}");
    }
}

#[test]
fn a_notice_from_the_cache_is_in_the_first_session_header() {
    let mut harness = Harness::start(Vec::new());
    // What the interface's entry does with `TuiOptions::update_notice`.
    harness
        .app
        .apply_appearance(AppearanceEvent::UpdateNotice(notice("0.3.0")));
    harness.pump_until_screen_shows("directory:");
    assert!(harness.screen().contains(FULL), "{}", harness.screen());
}

#[test]
fn a_settings_write_keeps_the_notice_and_both_conversations_show_it() {
    let mut harness = Harness::start(vec![assistant_events("Hi."), assistant_events("Four.")]);
    harness.notifier().notify(notice("0.3.0"));
    harness.pump_until_screen_shows(FULL);

    // `[tui]` as a settings write leaves it is taken whole.
    harness
        .app
        .adopt_tui_preferences(AgentTuiPreferences::default());
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(harness.screen().contains(FULL), "{}", harness.screen());

    // A side conversation opened after the notice shows it in its header.
    harness.answered("Hello.", "Hi.");
    harness.type_and_submit("/side What is 2 + 2?");
    harness.pump_until_screen_shows("Four.");
    harness.pump_until_turn_ends();
    assert!(harness.screen().contains(SHORT), "{}", harness.screen());

    // One sent while the side conversation has the focus reaches the
    // parked main one too.
    let newer = "Update available: 0.4.0";
    harness.notifier().notify(notice("0.4.0"));
    harness.pump_until_screen_shows(newer);
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Main conversation");
    assert!(harness.screen().contains(newer), "{}", harness.screen());
}

#[test]
fn a_narrow_header_wraps_the_notice_and_keeps_the_install_command() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        harness.tui.terminal.backend_mut().resize(36, super::HEIGHT);
        harness.pump_until_screen_shows("directory:");
        harness.notifier().notify(notice("0.3.0"));
        harness.pump_until_screen_shows(SHORT);
        let screen = harness.screen();
        assert!(screen.contains("hanafubuki-agent update"), "{screen}");
        assert!(harness.row_of(SHORT) < harness.row_of("╰"), "{screen}");
        for line in screen.lines().filter(|line| line.starts_with('│')) {
            assert!(line.trim_end().ends_with('│'), "{line}");
        }
    }
}

#[test]
fn new_and_resumed_conversations_keep_the_update_in_their_headers() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(vec![assistant_events("Hi.")], true, mode);
        harness.notifier().notify(notice("0.3.0"));
        harness.pump_until_screen_shows(FULL);
        harness.answered("Hello.", "Hi.");
        harness.type_and_submit("/new");
        harness.pump_until("no session", |harness| {
            harness.app.bridge.session_id().is_none()
        });
        harness.pump_until_screen_shows(FULL);
        assert!(harness.screen().contains(FULL), "{}", harness.screen());
        harness.type_and_submit("/resume");
        harness.pump_until_screen_shows("Resume a previous session");
        harness.key(KeyCode::Enter);
        harness.pump_until_screen_shows("Hi.");
        assert!(harness.screen().contains(FULL), "{}", harness.screen());
    }
}

#[test]
fn a_notice_received_in_settings_is_visible_on_return_to_the_conversation() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        harness.pump_until_screen_shows("directory:");
        harness.type_and_submit("/config");
        harness.pump_until_screen_shows("Settings");
        harness.notifier().notify(notice("0.3.0"));
        harness.pump_until("the update notice", |harness| {
            harness.transcript().contains(FULL)
        });
        harness.leave_settings();
        harness.pump_until_screen_shows(FULL);
        assert!(harness.row_of(FULL) < harness.row_of("╰"));
    }
}
