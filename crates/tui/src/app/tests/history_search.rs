//! Ctrl+R end to end over the standalone Agent, in Scrollback and
//! Fullscreen: a run finds what an earlier run sent (a message and a `!`
//! command) through the history store, and takes it as a draft; while a
//! turn runs, Esc and Ctrl+C close the search and leave the turn alone.

use std::time::Duration;

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::Harness;
use crate::transcript_mode::TranscriptMode;

impl Harness {
    /// The next run of the interface over this one's home (its Provider
    /// configured already), this one shut down.
    fn next_run(mut self, mode: TranscriptMode) -> Harness {
        let home = std::mem::replace(&mut self.home, tempfile::tempdir().unwrap());
        drop(self);
        Harness::start_in(home, Vec::new(), false, mode, |_| {}, None)
    }

    fn control(&mut self, c: char) {
        self.key_with(KeyCode::Char(c), KeyModifiers::CONTROL);
    }

    fn type_query(&mut self, query: &str) {
        for c in query.chars() {
            self.key(KeyCode::Char(c));
        }
    }
}

fn an_earlier_run_is_found(mode: TranscriptMode) {
    let mut harness = Harness::start_with(vec![assistant_events("Parsed.")], true, mode);
    harness.type_and_submit("an earlier message about parsers");
    harness.pump_until_screen_shows("Parsed.");
    harness.pump_until_turn_ends();
    harness.type_and_submit("!echo earlier-marker");
    harness.pump_until_screen_shows("You ran echo earlier-marker");
    harness.pump_until_turn_ends();

    let mut harness = harness.next_run(mode);
    harness.control('r');
    harness.pump_until_screen_shows("reverse-i-search:");
    harness.type_query("PARSERS");
    harness.pump_until_screen_shows("› an earlier message about parsers");
    let screen = harness.screen();
    assert!(
        screen.contains("reverse-i-search: PARSERS  enter accept · esc cancel"),
        "{screen}"
    );
    harness.control('u');
    harness.type_query("!ech");
    harness.pump_until_screen_shows("› !echo earlier-marker");

    // Up steps to an older match; there is none, and the match stays.
    harness.key(KeyCode::Up);
    harness.key(KeyCode::Enter);
    harness.pump_until("the search to close", |harness| {
        !harness.screen().contains("reverse-i-search:")
    });
    assert_eq!(
        harness.app.chat_widget.composer_text(),
        "!echo earlier-marker"
    );
}

#[test]
fn ctrl_r_finds_what_an_earlier_run_sent_in_scrollback() {
    an_earlier_run_is_found(TranscriptMode::Terminal);
}

#[test]
fn ctrl_r_finds_what_an_earlier_run_sent_in_fullscreen() {
    an_earlier_run_is_found(TranscriptMode::Owned);
}

fn the_search_closes_without_touching_a_running_turn(mode: TranscriptMode) {
    let mut harness = Harness::start_timed(
        vec![(assistant_events("Slow answer."), Duration::from_secs(60))],
        true,
        mode,
    );
    harness.type_and_submit("Take your time.");
    harness.pump_until("the turn to run", |harness| {
        harness.app.chat_widget.is_task_running()
    });
    for close in ['\u{1b}', 'c'] {
        harness.control('r');
        harness.type_query("time");
        harness.pump_until_screen_shows("› Take your time.");
        if close == 'c' {
            harness.control('c');
        } else {
            harness.key(KeyCode::Esc);
        }
        harness.pump_until("the search to close", |harness| {
            !harness.screen().contains("reverse-i-search:")
        });
        let screen = harness.screen();
        assert!(harness.app.chat_widget.is_task_running(), "{screen}");
        assert!(!screen.contains("again to quit"), "{screen}");
        assert!(!screen.contains("Interrupted"), "{screen}");
        assert_eq!(harness.app.chat_widget.composer_text(), "");
    }
}

#[test]
fn esc_and_ctrl_c_close_the_search_not_the_turn_in_scrollback() {
    the_search_closes_without_touching_a_running_turn(TranscriptMode::Terminal);
}

#[test]
fn esc_and_ctrl_c_close_the_search_not_the_turn_in_fullscreen() {
    the_search_closes_without_touching_a_running_turn(TranscriptMode::Owned);
}
