//! The keys `?` lists, end to end over the standalone Agent, in Scrollback
//! and Fullscreen: Shift+Tab switches the mode, Alt+, and Alt+. step the
//! effort through what `/effort` offers (and are refused during a turn as
//! it is), Alt+Left and Alt+Right step through the session and its
//! subagents on an empty composer.

use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{AgentProviderModelCatalogRequest, AgentSessionMode};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::subagents::spawn_and_wait;
use super::Harness;
use crate::commands::effort::DURING_TASK;
use crate::transcript_mode::TranscriptMode;

impl Harness {
    fn session_mode(&self) -> AgentSessionMode {
        let agent = self.runtime.slot().agent().unwrap();
        agent
            .snapshot(self.app.bridge.session_id().unwrap())
            .unwrap()
            .mode
    }

    fn session_effort(&self) -> Option<String> {
        let agent = self.runtime.slot().agent().unwrap();
        agent
            .snapshot(self.app.bridge.session_id().unwrap())
            .unwrap()
            .reasoning_effort
            .map(|effort| effort.to_string())
    }

    fn alt(&mut self, code: KeyCode) {
        self.key_with(code, KeyModifiers::ALT);
    }
}

#[test]
fn shift_tab_switches_the_mode_in_both_presentations() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        harness.key(KeyCode::Char('?'));
        let shown = harness.pump_until_screen_shows("Keyboard shortcuts");
        assert!(shown.contains("Change mode"), "{shown}");
        assert!(shown.contains("/keymap customize"), "{shown}");
        harness.key(KeyCode::Char('?'));

        harness.key(KeyCode::BackTab);
        harness.pump_until_screen_shows("Mode set to plan");
        assert_eq!(harness.session_mode(), AgentSessionMode::Plan);
        harness.key_with(KeyCode::Tab, KeyModifiers::SHIFT);
        harness.pump_until_screen_shows("Mode set to default");
        assert_eq!(harness.session_mode(), AgentSessionMode::Default);
    }
}

/// The listing reports `low` and `high`: an unset effort starts at the
/// first, and the steps stop at either end.
#[test]
fn alt_comma_and_period_step_the_effort_through_the_listed_levels() {
    let listing = serde_json::json!({"data": [
        {"id": "fixture-model", "reasoning": {"supported_efforts": ["high", "low"]}}
    ]});
    let mut harness = Harness::owned(vec![listing.to_string().into_bytes()]);
    let agent = harness.runtime.slot().agent().unwrap();
    let call = agent.clone();
    agent
        .runtime()
        .block_on(async move {
            call.list_provider_models(AgentProviderModelCatalogRequest::default())
                .await
        })
        .unwrap()
        .unwrap();

    harness.alt(KeyCode::Char('.'));
    harness.pump_until_screen_shows("Reasoning effort set to high");
    assert_eq!(harness.session_effort().as_deref(), Some("high"));
    harness.alt(KeyCode::Char('.'));
    harness.pump_until_screen_shows("Reasoning is already at the highest level (High).");
    harness.alt(KeyCode::Char(','));
    harness.pump_until_screen_shows("Reasoning effort set to low");
    assert_eq!(harness.session_effort().as_deref(), Some("low"));
    harness.alt(KeyCode::Char(','));
    harness.pump_until_screen_shows("Reasoning is already at the lowest level (Low).");
    assert_eq!(harness.session_effort().as_deref(), Some("low"));
}

/// Two presses before the first step is set (a key held) take two steps.
#[test]
fn quick_effort_steps_each_take_a_level() {
    let listing = serde_json::json!({"data": [
        {"id": "fixture-model", "reasoning": {"supported_efforts": ["low", "medium", "high"]}}
    ]});
    let mut harness = Harness::start(vec![listing.to_string().into_bytes()]);
    let agent = harness.runtime.slot().agent().unwrap();
    let call = agent.clone();
    agent
        .runtime()
        .block_on(async move {
            call.list_provider_models(AgentProviderModelCatalogRequest::default())
                .await
        })
        .unwrap()
        .unwrap();
    harness.type_and_submit("/effort low");
    harness.pump_until_screen_shows("Reasoning effort set to low");

    harness.alt(KeyCode::Char('.'));
    harness.alt(KeyCode::Char('.'));
    harness.pump_until("two steps", |harness| {
        harness.session_effort().as_deref() == Some("high")
    });
    assert!(
        !harness.transcript().contains("was not changed"),
        "{}",
        harness.transcript()
    );
}

#[test]
fn an_effort_step_without_levels_or_during_a_turn_changes_nothing() {
    let mut harness = Harness::start_timed(
        vec![(assistant_events("Done."), Duration::from_secs(2))],
        true,
        TranscriptMode::Terminal,
    );
    harness.alt(KeyCode::Char('.'));
    harness.pump_until_screen_shows("No reasoning levels are known for this model");
    assert_eq!(harness.session_effort(), None);

    harness.type_and_submit("Take your time.");
    harness.pump_until("the turn to start", |harness| {
        harness.app.chat_widget.is_task_running()
    });
    harness.alt(KeyCode::Char(','));
    harness.pump_until_screen_shows("Change the effort after this turn");
    assert!(
        harness.transcript().contains(DURING_TASK),
        "{}",
        harness.transcript()
    );
    harness.pump_until_turn_ends();
    assert_eq!(harness.session_effort(), None);
}

#[test]
fn alt_arrows_step_through_the_session_and_its_subagents() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        step_through_the_subagents(mode);
    }
}

fn step_through_the_subagents(mode: TranscriptMode) {
    let mut harness = Harness::start_with(
        vec![
            spawn_and_wait("Count the files."),
            assistant_events("Twelve files."),
            assistant_events("There are twelve."),
        ],
        true,
        mode,
    );
    let parent = harness.answered("Delegate the count.", "There are twelve.");
    let [child] = harness.subagents_of(parent)[..] else {
        panic!("one subagent");
    };

    harness.alt(KeyCode::Right);
    harness.pump_until_screen_shows("Subagent counter · from Delegate the count.");
    assert_eq!(harness.app.bridge.session_id(), Some(child));
    // Past the last subagent: the main session again.
    harness.alt(KeyCode::Right);
    harness.pump_until("the main session shown", |harness| {
        harness.app.bridge.session_id() == Some(parent)
    });
    assert!(!harness.app.workspace.second_focused());
    // And back, round the other way.
    harness.alt(KeyCode::Left);
    harness.pump_until("the subagent shown", |harness| {
        harness.app.bridge.session_id() == Some(child)
    });
    harness.alt(KeyCode::Left);
    harness.pump_until("the main session shown", |harness| {
        harness.app.bridge.session_id() == Some(parent)
    });

    // With text in the composer, Alt+Left moves by word.
    harness.key(KeyCode::Char('x'));
    std::thread::sleep(Duration::from_millis(100));
    harness.app.draw(&mut harness.tui).unwrap();
    harness.alt(KeyCode::Left);
    // A step would be shown once the subagents are listed: give it time.
    let until = Instant::now() + Duration::from_millis(500);
    harness.pump_until("a moment", |_| Instant::now() >= until);
    assert_eq!(harness.app.chat_widget.composer_text(), "x");
    assert_eq!(harness.app.bridge.session_id(), Some(parent));
}

#[test]
fn alt_right_says_when_the_session_has_no_subagents() {
    let mut harness = Harness::owned(vec![assistant_events("Nothing to delegate.")]);
    harness.alt(KeyCode::Right);
    harness.pump_until_screen_shows("This session has no subagents.");
    harness.answered("Hello.", "Nothing to delegate.");
    harness.alt(KeyCode::Left);
    harness.pump_until("the second notice", |harness| {
        harness
            .transcript()
            .matches("This session has no subagents.")
            .count()
            == 2
    });
}
