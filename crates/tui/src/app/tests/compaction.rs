//! Context compactions in the conversation, end to end: every install shows
//! one "Context compacted" cell, whether `/compact` asked for it or a turn
//! took it from the background, and a resumed session replays it once;
//! `/status` shows a compaction running in the background; a failed one
//! shows a warning when a turn ran it, and `/compact`'s reply otherwise.

use std::time::Duration;

use hanafubuki_agent::test_support::{assistant_events, tool_events, with_input_tokens};
use hanafubuki_agent::{
    AgentBackgroundCompactionState, AgentCompactionSettings, AgentCompactionStrategy,
    AgentSettingsPatch,
};

use super::Harness;
use crate::transcript_mode::TranscriptMode;

const COMPACTED: &str = "Context compacted";
const FIRST_ANSWER: &str = "The first answer is long. ";

impl Harness {
    /// The `background` strategy over a window of 100,000 tokens. The
    /// first two responses are the exchanges [`Harness::two_long_exchanges`]
    /// has; the second reports 70,000 input tokens, between the background
    /// threshold (65%) and the foreground one (80%), so the session starts a
    /// compaction of the first exchange in the background, whose summary is
    /// the third response.
    fn background(responses: Vec<(Vec<u8>, Duration)>) -> Self {
        let mut all = vec![
            (assistant_events(&FIRST_ANSWER.repeat(100)), Duration::ZERO),
            (
                with_input_tokens(assistant_events("Done."), 70_000),
                Duration::ZERO,
            ),
        ];
        all.extend(responses);
        Self::start_configured(all, true, TranscriptMode::Terminal, |config| {
            config
                .mutate_providers(1, |registry| {
                    registry.model_policy.context_window_tokens = Some(100_000);
                    Ok(())
                })
                .unwrap();
            config
                .write_settings(AgentSettingsPatch {
                    compaction: Some(AgentCompactionSettings {
                        strategy: AgentCompactionStrategy::Background,
                        ..AgentCompactionSettings::default()
                    }),
                    ..AgentSettingsPatch::default()
                })
                .unwrap();
        })
    }

    fn two_long_exchanges(&mut self) {
        self.exchange("Hello.", FIRST_ANSWER.trim_end());
        self.exchange("Go on.", "Done.");
    }

    fn background_compaction(&self) -> Option<AgentBackgroundCompactionState> {
        let agent = self.runtime.slot().agent().unwrap();
        let session_id = self.app.bridge.session_id().unwrap();
        agent.snapshot(session_id).unwrap().background_compaction
    }

    fn compacted_cells(&self) -> usize {
        self.transcript().matches(COMPACTED).count()
    }

    /// Leaves the session and resumes it, which replays its transcript.
    fn resume_again(&mut self, last_answer: &str) {
        let session_id = self.app.bridge.session_id().unwrap();
        self.type_and_submit("/new");
        self.pump_until("no session", |harness| {
            harness.app.bridge.session_id().is_none()
        });
        assert_eq!(self.compacted_cells(), 0);
        self.type_and_submit(&format!("/resume {}", session_id.value()));
        self.pump_until("the resumed transcript", |harness| {
            harness.transcript().contains(last_answer)
        });
    }
}

#[test]
fn a_compaction_taken_from_the_background_shows_once_in_the_turn_that_took_it() {
    let mut harness = Harness::background(vec![
        (assistant_events("The user said hello."), Duration::ZERO),
        (assistant_events("Still here."), Duration::ZERO),
    ]);
    harness.two_long_exchanges();
    harness.pump_until("the background compaction to be ready", |harness| {
        harness.background_compaction() == Some(AgentBackgroundCompactionState::Ready)
    });
    assert_eq!(harness.compacted_cells(), 0);

    harness.exchange("Are you there?", "Still here.");
    let transcript = harness.transcript();
    assert_eq!(harness.compacted_cells(), 1, "{transcript}");
    let question = transcript.find("Are you there?").unwrap();
    let compacted = transcript.find(COMPACTED).unwrap();
    let answer = transcript.find("Still here.").unwrap();
    assert!(question < compacted && compacted < answer, "{transcript}");
    let agent = harness.runtime.slot().agent().unwrap();
    let compaction = agent
        .snapshot(harness.app.bridge.session_id().unwrap())
        .unwrap()
        .compaction
        .unwrap();
    assert_eq!(
        serde_json::to_value(compaction.phase).unwrap(),
        "background"
    );
    assert_eq!(harness.server.requests().len(), 4);

    harness.resume_again("Still here.");
    assert_eq!(harness.compacted_cells(), 1, "{}", harness.transcript());
}

#[test]
fn status_shows_a_compaction_running_in_the_background() {
    // The summary is held, so the compaction is still running when `/status`
    // reads the session; the Agent's shutdown cancels it.
    let mut harness = Harness::background(vec![(
        assistant_events("The user said hello."),
        Duration::from_secs(30),
    )]);
    harness.two_long_exchanges();
    harness.pump_until("the background compaction to run", |harness| {
        harness.background_compaction() == Some(AgentBackgroundCompactionState::Running)
    });
    harness.type_and_submit("/status");
    let screen = harness.pump_until_screen_shows("compacting in the background");
    assert!(screen.contains("Compaction:"), "{screen}");
    assert_eq!(harness.compacted_cells(), 0);
}

#[test]
fn compact_shows_one_cell_and_a_resumed_session_replays_it_once() {
    let mut harness = Harness::start(vec![
        assistant_events("The answer is 42."),
        assistant_events("The user asked for the answer."),
        assistant_events("Still here."),
    ]);
    harness.exchange("What is the answer?", "The answer is 42.");
    harness.type_and_submit("/compact");
    harness.pump_until("the compaction's cell", |harness| {
        harness.compacted_cells() > 0
    });
    // Whatever `/compact` still had to say has arrived by the next answer.
    harness.exchange("Are you there?", "Still here.");
    assert_eq!(harness.compacted_cells(), 1, "{}", harness.transcript());

    harness.resume_again("Still here.");
    assert_eq!(harness.compacted_cells(), 1, "{}", harness.transcript());
}

#[test]
fn a_compaction_that_fails_in_a_turn_shows_a_warning_and_compacts_reply_shows_once() {
    const WARNING: &str = "Context compaction failed";
    // A summary that calls a tool is refused, so both compactions fail.
    let mut harness = Harness::start_configured(
        vec![
            (
                with_input_tokens(assistant_events("The answer is 42."), 90_000),
                Duration::ZERO,
            ),
            (tool_events("skills_list", "{}"), Duration::ZERO),
            (tool_events("skills_list", "{}"), Duration::ZERO),
        ],
        true,
        TranscriptMode::Terminal,
        |config| {
            config
                .mutate_providers(1, |registry| {
                    registry.model_policy.context_window_tokens = Some(100_000);
                    Ok(())
                })
                .unwrap();
        },
    );
    harness.exchange("What is the answer?", "The answer is 42.");

    harness.type_and_submit("/compact");
    harness.pump_until_screen_shows("the conversation was not compacted");
    assert!(!harness.transcript().contains(WARNING));

    // Over the threshold, the next turn compacts first, and fails with it.
    harness.type_and_submit("Are you there?");
    harness.pump_until_screen_shows(WARNING);
    harness.pump_until_turn_ends();
    let transcript = harness.transcript();
    assert_eq!(transcript.matches(WARNING).count(), 1, "{transcript}");
    assert!(
        transcript.contains("local compaction returned a forbidden output item"),
        "{transcript}"
    );
    harness.type_and_submit("/warnings");
    let list = harness.pump_until_screen_shows("1 compaction");
    assert!(list.contains(&format!("[compaction] {WARNING}")), "{list}");
}
