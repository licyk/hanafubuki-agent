//! `/review`, `/goal` and `/hooks` end to end over the standalone Agent:
//! a review over the scripted model reported into the conversation, a
//! message held for a review's end and one cancelled, the goal set, replaced and paused, and a hook trusted
//! from the browser after the start reported it.

use std::time::Duration;

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{
    AgentGoalStatus, AgentSessionId, AgentSessionListRequest, AgentSessionOrigin,
    AgentSessionParentFilter, HookTrust,
};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::Harness;
use crate::transcript_mode::TranscriptMode;

const FINDINGS: &str = r#"{
  "findings": [
    {
      "title": "[P1] Check the length before indexing",
      "body": "`items[0]` panics on an empty list.",
      "confidence_score": 0.8,
      "priority": 1,
      "code_location": {
        "absolute_file_path": "/work/src/lib.rs",
        "line_range": {"start": 10, "end": 12}
      }
    }
  ],
  "overall_correctness": "patch is incorrect",
  "overall_explanation": "One panic on empty input.",
  "overall_confidence_score": 0.7
}"#;

impl Harness {
    /// Sends `text` and waits for the answer `answer` and the turn's end.
    pub(super) fn answered(&mut self, text: &str, answer: &str) -> AgentSessionId {
        self.type_and_submit(text);
        self.pump_until_screen_shows(answer);
        self.pump_until_turn_ends();
        self.app.bridge.session_id().unwrap()
    }

    fn review_children(&self, parent: AgentSessionId) -> usize {
        let agent = self.runtime.slot().agent().unwrap();
        agent
            .list_sessions_page(AgentSessionListRequest {
                parent: AgentSessionParentFilter::ChildrenOf(parent),
                ..AgentSessionListRequest::default()
            })
            .unwrap()
            .sessions
            .iter()
            .filter(|child| child.origin == Some(AgentSessionOrigin::Review))
            .count()
    }
}

#[test]
fn a_review_needs_a_conversation_and_reports_its_findings_into_it() {
    let mut harness = Harness::start(vec![assistant_events("Hi."), assistant_events(FINDINGS)]);
    harness.type_and_submit("/review");
    harness.pump_until_screen_shows("Send a message first; a review reports back");
    assert_eq!(harness.app.bridge.session_id(), None);

    let parent = harness.answered("Hello.", "Hi.");
    harness.type_and_submit("/review");
    harness.pump_until_screen_shows("Select a review preset");
    harness.key(KeyCode::Enter);
    harness.pump_until("the review's findings", |harness| {
        harness
            .transcript()
            .contains("[P1] Check the length before indexing")
    });
    let transcript = harness.transcript();
    for text in [
        ">> Code review started: current changes <<",
        "<< Code review finished >>",
        "Verdict: patch is incorrect",
        "/work/src/lib.rs:10-12",
    ] {
        assert!(transcript.contains(text), "{text:?} in:\n{transcript}");
    }
    harness.pump_until_turn_ends();
    assert_eq!(harness.review_children(parent), 1);
}

#[test]
fn a_message_sent_during_a_review_starts_the_turn_after_it() {
    let mut harness = Harness::start_timed(
        vec![
            (assistant_events("Hi."), Duration::ZERO),
            (assistant_events(FINDINGS), Duration::from_secs(2)),
            (assistant_events("Noted."), Duration::ZERO),
        ],
        true,
        TranscriptMode::Terminal,
    );
    harness.answered("Hello.", "Hi.");
    harness.type_and_submit("/review look at the error handling");
    harness.pump_until("the review to sample", |harness| {
        harness.server.requests().len() == 2
    });
    harness.type_and_submit("What did it find?");
    harness.pump_until_screen_shows("Messages to be submitted at end of turn");
    harness.pump_until_screen_shows("Noted.");
    let transcript = harness.transcript();
    let finished = transcript
        .find("<< Code review finished >>")
        .expect(&transcript);
    assert!(
        finished < transcript.find("› What did it find?").expect(&transcript),
        "{transcript}"
    );
    assert_eq!(harness.server.requests().len(), 3);
}

#[test]
fn esc_keeps_a_review_holding_messages_and_ctrl_c_cancels_it_into_the_composer() {
    let mut harness = Harness::start_timed(
        vec![
            (assistant_events("Hi."), Duration::ZERO),
            (assistant_events(FINDINGS), Duration::from_secs(30)),
        ],
        true,
        TranscriptMode::Terminal,
    );
    harness.answered("Hello.", "Hi.");
    harness.type_and_submit("/review look at the error handling");
    harness.pump_until("the review to sample", |harness| {
        harness.server.requests().len() == 2
    });
    assert!(harness
        .screen()
        .contains("Reviewing look at the error handling"));

    harness.type_and_submit("What did it find?");
    harness.pump_until_screen_shows("Messages to be submitted at end of turn");
    // Esc would send them at once, which a review cannot take: it says so.
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Messages sent during /review wait for it to end");
    assert!(harness.app.chat_widget.is_task_running());

    harness.key_with(KeyCode::Char('c'), KeyModifiers::CONTROL);
    harness.pump_until("the review to be cancelled", |harness| {
        harness.transcript().contains("<< Code review cancelled >>")
    });
    // The message is back in the composer, unsent.
    let screen = harness.pump_until_screen_shows("› What did it find?");
    assert!(!screen.contains("Messages to be submitted"), "{screen}");
    assert!(!harness.transcript().contains("What did it find?"));
    assert_eq!(harness.server.requests().len(), 2);
}

#[test]
fn goal_sets_replaces_after_asking_and_pauses_the_sessions_goal() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/statusline goal --no-colors");
    harness.type_and_submit("/goal --tokens 50k Ship the release");
    harness.pump_until_screen_shows("Goal set: Ship the release");
    let session_id = harness.app.bridge.session_id().unwrap();
    let goal = |harness: &Harness| {
        let agent = harness.runtime.slot().agent().unwrap();
        let reader = agent.clone();
        agent
            .runtime()
            .block_on(async move { reader.goal(session_id).await })
            .unwrap()
            .unwrap()
            .unwrap()
    };
    let set = goal(&harness);
    assert_eq!(set.objective, "Ship the release");
    assert_eq!(set.token_budget, Some(50_000));
    // No turn runs, so no event: the status line takes the call's goal.
    harness.pump_until_screen_shows("Goal: Ship the release (active)");

    harness.type_and_submit("/goal Fix the build");
    harness.pump_until_screen_shows("Replace goal?");
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Goal set: Fix the build");
    assert_eq!(goal(&harness).objective, "Fix the build");

    harness.type_and_submit("/goal pause");
    harness.pump_until_screen_shows("Goal paused");
    assert_eq!(goal(&harness).status, AgentGoalStatus::Paused);
    harness.pump_until_screen_shows("Goal: Fix the build (paused)");
}

#[test]
fn hooks_waiting_at_start_are_reported_and_trusted_from_the_browser() {
    let mut harness = Harness::start(Vec::new());
    // The check at start runs once the app loop does.
    std::fs::write(
        harness.home.path().join("hooks.json"),
        r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "true"}]}]}}"#,
    )
    .unwrap();
    harness.pump_until_screen_shows("1 hook needs review: /hooks");

    harness.type_and_submit("/hooks");
    harness.pump_until_screen_shows("1 need review");
    for _ in 0..4 {
        harness.key(KeyCode::Down);
    }
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("new, review required");
    harness.key(KeyCode::Char('t'));
    harness.pump_until_screen_shows("trusted · on");

    let agent = harness.runtime.slot().agent().unwrap();
    let reader = agent.clone();
    let hooks = agent
        .runtime()
        .block_on(async move { reader.hooks_list().await })
        .unwrap()
        .unwrap();
    assert_eq!(hooks.hooks.len(), 1);
    assert_eq!(hooks.hooks[0].trust, HookTrust::Trusted);
}
