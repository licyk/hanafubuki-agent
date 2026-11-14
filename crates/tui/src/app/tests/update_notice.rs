// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The update notice end to end: from the command line's cache at launch
//! it is in the first frame; from its check mid-session (the notifier) it
//! appears on the footer's row without moving the composer, in both
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
fn a_notice_from_the_check_appears_on_the_footer_row_without_moving_the_composer() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        harness.pump_until_screen_shows(STATUS);
        let composer = harness.composer_row();
        let footer = harness.row_of(STATUS);
        assert_eq!(harness.row_of("Update available"), None);

        harness.notifier().notify(notice("0.3.0"));
        harness.pump_until_screen_shows(FULL);

        assert_eq!(harness.composer_row(), composer, "{mode:?}");
        assert_eq!(harness.row_of(FULL), footer, "{mode:?}");
        assert_eq!(harness.row_of(STATUS), footer, "{mode:?}");
    }
}

#[test]
fn a_notice_from_the_cache_is_in_the_first_frame() {
    let mut harness = Harness::start(Vec::new());
    // What the interface's entry does with `TuiOptions::update_notice`.
    harness
        .app
        .apply_appearance(AppearanceEvent::UpdateNotice(notice("0.3.0")));
    harness.app.draw(&mut harness.tui).unwrap();
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

    // A side conversation opened after the notice shows it; beside the
    // session's title and `← for agents`, in its short form.
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
