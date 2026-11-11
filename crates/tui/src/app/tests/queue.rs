//! Messages sent while the Agent works, end to end over the standalone
//! Agent: Enter steers the running turn at its next step, a steer that
//! races the turn's end becomes the next turn, Esc interrupts and sends
//! the waiting steers as one turn, Tab queues a message for the turns
//! after (drained after a completed and a failed turn, put back in the
//! composer by an interrupt), Shift+Left edits the last queued message,
//! a parked conversation's queue drains on its own, and a subagent shown
//! steers and queues as the session does; and what comes
//! before them: a scrolled conversation's Esc, a side conversation's own
//! turn, the leader's Tab.

use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{AgentSessionListRequest, AgentTurnPhase, ConversationItem};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::Harness;
use crate::transcript_mode::TranscriptMode;

/// A model stream that completes with no answer: the turn fails.
fn failing_events() -> Vec<u8> {
    concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-empty\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-empty\",\"end_turn\":true}}\n\n"
    )
    .as_bytes()
    .to_vec()
}

fn slow(events: Vec<u8>, seconds: u64) -> (Vec<u8>, Duration) {
    (events, Duration::from_secs(seconds))
}

fn now(events: Vec<u8>) -> (Vec<u8>, Duration) {
    (events, Duration::ZERO)
}

impl Harness {
    fn timed(responses: Vec<(Vec<u8>, Duration)>) -> Self {
        Self::start_timed(responses, true, TranscriptMode::Terminal)
    }

    /// Types `text` into the composer without sending it.
    fn type_draft(&mut self, text: &str) {
        for character in text.chars() {
            self.key(KeyCode::Char(character));
        }
        std::thread::sleep(Duration::from_millis(100));
        self.app.draw(&mut self.tui).unwrap();
    }

    fn type_and_queue(&mut self, text: &str) {
        self.type_draft(text);
        self.key(KeyCode::Tab);
    }

    /// Waits until the chat has seen the turn start, so Enter steers it.
    fn pump_until_running(&mut self) {
        self.pump_until("the turn to run", |harness| {
            harness.app.chat_widget.is_task_running()
        });
    }

    /// The person's messages the session holds, oldest first, a steering
    /// one marked.
    fn saved_prompts(&self) -> Vec<String> {
        let agent = self.runtime.slot().agent().unwrap();
        let session_id = self.app.bridge.session_id().unwrap();
        let mut prompts: Vec<String> = agent
            .open_session(session_id, 50)
            .unwrap()
            .transcript
            .items
            .into_iter()
            .filter_map(|item| match item.content {
                ConversationItem::UserMessage { text, .. } => Some(text),
                ConversationItem::SteeringMessage { text, .. } => Some(format!("steering: {text}")),
                _ => None,
            })
            .collect();
        prompts.reverse();
        prompts
    }

    /// The phase of the session's latest turn, as the Agent has it.
    fn latest_turn_phase(&self) -> Option<AgentTurnPhase> {
        let agent = self.runtime.slot().agent().unwrap();
        let session_id = self.app.bridge.session_id()?;
        agent
            .list_sessions_page(AgentSessionListRequest::default())
            .unwrap()
            .sessions
            .into_iter()
            .find(|summary| summary.session_id == session_id)?
            .latest_turn_phase
    }

    /// `texts` appear in the transcript in this order.
    fn assert_in_order(&self, texts: &[&str]) {
        let transcript = self.transcript();
        let mut from = 0;
        for text in texts {
            let at = transcript[from..]
                .find(text)
                .unwrap_or_else(|| panic!("{text:?} not in order:\n{transcript}"));
            from += at + text.len();
        }
    }
}

#[test]
fn enter_steers_the_running_turn_which_reads_it_at_its_next_step() {
    let mut harness = Harness::timed(vec![
        slow(assistant_events("Working on it."), 2),
        now(assistant_events("Steered.")),
    ]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_submit("Also check the tests.");
    let waiting = harness.pump_until_screen_shows("Messages to be submitted after next tool call");
    assert!(waiting.contains("↳ Also check the tests."), "{waiting}");
    // Not written until the turn reads it.
    assert!(!harness.transcript().contains("Also check the tests."));

    harness.pump_until_screen_shows("Steered.");
    harness.pump_until_turn_ends();
    harness.assert_in_order(&[
        "› Start.",
        "Working on it.",
        "› Also check the tests.",
        "Steered.",
    ]);
    let requests = harness.server.requests();
    assert_eq!(requests.len(), 2);
    assert!(String::from_utf8_lossy(&requests[1].body).contains("Also check the tests."));
    assert_eq!(
        harness.saved_prompts(),
        vec!["Start.", "steering: Also check the tests."]
    );
    assert!(!harness.screen().contains("Messages to be submitted"));
}

#[test]
fn a_steer_racing_the_turns_end_becomes_the_next_turn() {
    let mut harness = Harness::timed(vec![
        now(assistant_events("First answer.")),
        now(assistant_events("Second answer.")),
    ]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    // The turn ends in the Agent while the chat, not yet told, still
    // steers it.
    let deadline = Instant::now() + Duration::from_secs(30);
    while harness.latest_turn_phase() != Some(AgentTurnPhase::Completed) {
        assert!(Instant::now() < deadline, "the turn did not complete");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(harness.app.chat_widget.is_task_running());
    harness.type_and_submit("Late message.");

    harness.pump_until_screen_shows("Second answer.");
    harness.pump_until_turn_ends();
    harness.assert_in_order(&["First answer.", "› Late message.", "Second answer."]);
    assert_eq!(harness.saved_prompts(), vec!["Start.", "Late message."]);
}

#[test]
fn esc_interrupts_and_sends_the_waiting_steers_as_one_turn_which_can_be_edited() {
    let mut harness = Harness::timed(vec![
        slow(assistant_events("Too slow."), 3),
        now(assistant_events("Resent.")),
    ]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_submit("First steer.");
    harness.type_and_submit("Second steer.");
    harness.pump_until_screen_shows("↳ Second steer.");
    harness.type_draft("a draft");

    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Resent.");
    harness.pump_until_turn_ends();
    harness.assert_in_order(&[
        "› Start.",
        "Model interrupted to submit steer instructions.",
        "› First steer.",
        "Second steer.",
        "Resent.",
    ]);
    assert!(!harness.transcript().contains("Too slow."));
    assert_eq!(harness.app.chat_widget.composer_text(), "a draft");
    assert_eq!(
        harness.saved_prompts(),
        vec!["Start.", "First steer.\nSecond steer."]
    );

    // The message sent at once is one the conversation holds: it edits.
    harness.key_with(KeyCode::Char('c'), KeyModifiers::CONTROL);
    harness.key(KeyCode::Esc);
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Browsing");
    harness.key(KeyCode::Enter);
    assert_eq!(
        harness.app.chat_widget.composer_text(),
        "First steer.\nSecond steer."
    );
    assert_eq!(harness.saved_prompts(), vec!["Start."]);
}

#[test]
fn tab_queues_each_message_for_a_turn_of_its_own_after_the_running_one() {
    let mut harness = Harness::timed(vec![
        slow(assistant_events("First."), 2),
        now(assistant_events("Second.")),
        now(assistant_events("Third.")),
    ]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_queue("Queued one.");
    harness.type_and_queue("Queued two.");
    let queued = harness.pump_until_screen_shows("Queued follow-up inputs");
    assert!(queued.contains("↳ Queued two."), "{queued}");

    harness.pump_until_screen_shows("Third.");
    harness.pump_until_turn_ends();
    harness.assert_in_order(&[
        "First.",
        "› Queued one.",
        "Second.",
        "› Queued two.",
        "Third.",
    ]);
    assert_eq!(
        harness.saved_prompts(),
        vec!["Start.", "Queued one.", "Queued two."]
    );
}

#[test]
fn a_failed_turn_goes_on_with_the_queue() {
    let mut harness = Harness::timed(vec![
        slow(failing_events(), 2),
        now(assistant_events("Recovered.")),
    ]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_queue("After the failure.");
    harness.pump_until_screen_shows("Recovered.");
    harness.assert_in_order(&["■", "› After the failure.", "Recovered."]);
}

#[test]
fn an_interrupt_puts_the_queue_back_in_the_composer_and_sends_nothing() {
    let mut harness = Harness::timed(vec![slow(assistant_events("Too slow."), 3)]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_queue("Queued.");
    harness.pump_until_screen_shows("Queued follow-up inputs");
    // No steer waits: Esc interrupts, as Ctrl-C does.
    harness.key(KeyCode::Esc);
    harness.pump_until_turn_ends();
    assert_eq!(harness.app.chat_widget.composer_text(), "Queued.");
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.screen().contains("Queued follow-up inputs"));
    harness.pump_until("the interruption", |harness| {
        harness.transcript().contains("Conversation interrupted")
    });
    assert!(!harness.transcript().contains("Queued."));
    assert_eq!(harness.server.requests().len(), 1);
}

#[test]
fn shift_left_edits_the_last_queued_message_but_a_selection_keeps_its_keys() {
    let mut harness = Harness::start_timed(
        vec![slow(assistant_events("Done."), 3)],
        true,
        TranscriptMode::Owned,
    );
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_queue("Queued one.");
    harness.type_and_queue("Queued two.");
    harness.pump_until_screen_shows("↳ Queued two.");

    // A keyboard selection in the conversation takes Shift+Left.
    harness.key_with(KeyCode::Char(' '), KeyModifiers::CONTROL);
    assert!(harness.app.transcript_view.has_active_interaction());
    harness.key_with(KeyCode::Left, KeyModifiers::SHIFT);
    assert!(harness.app.chat_widget.composer_is_empty());
    harness.key(KeyCode::Esc);
    assert!(!harness.app.transcript_view.has_active_interaction());
    assert!(harness.app.chat_widget.is_task_running());

    harness.key_with(KeyCode::Left, KeyModifiers::SHIFT);
    assert_eq!(harness.app.chat_widget.composer_text(), "Queued two.");
    harness.app.draw(&mut harness.tui).unwrap();
    let screen = harness.screen();
    assert!(screen.contains("↳ Queued one."), "{screen}");
    assert!(!screen.contains("↳ Queued two."), "{screen}");
}

#[test]
fn a_parked_conversations_queue_drains_while_the_side_one_has_the_focus() {
    let mut harness = Harness::timed(vec![
        now(assistant_events("Hi.")),
        slow(assistant_events("Main done."), 2),
        now(assistant_events("Queued answer.")),
    ]);
    harness.answered("Hello.", "Hi.");
    harness.type_and_submit("Work.");
    harness.pump_until_running();
    harness.type_and_queue("Queued for the main one.");
    harness.pump_until_screen_shows("Queued follow-up inputs");

    harness.type_and_submit("/side");
    harness.pump_until("the side conversation", |harness| {
        harness.app.workspace.second_focused()
    });
    // The side conversation has a queue of its own, empty.
    assert!(!harness.screen().contains("Queued follow-up inputs"));
    harness.pump_until("the main queue to drain", |harness| {
        harness.server.requests().len() == 3
    });

    harness.key(KeyCode::Esc);
    harness.pump_until("the main conversation", |harness| {
        !harness.app.workspace.second_focused()
    });
    harness.pump_until("the queued answer", |harness| {
        harness.transcript().contains("Queued answer.")
    });
    harness.assert_in_order(&["Main done.", "› Queued for the main one.", "Queued answer."]);
}

#[test]
fn esc_returns_a_scrolled_conversation_to_the_bottom_before_it_interrupts() {
    let mut harness = Harness::start_timed(
        vec![slow(assistant_events("Too slow."), 3)],
        true,
        TranscriptMode::Owned,
    );
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    for note in 0..60 {
        harness
            .app
            .chat_widget
            .add_info_message(format!("note {note:02}"));
    }
    harness.pump_until_screen_shows("note 59");
    harness.key(KeyCode::PageUp);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(harness.screen().contains("Back to bottom"));

    harness.key(KeyCode::Esc);
    let latest = harness.pump_until_screen_shows("note 59");
    assert!(!latest.contains("Back to bottom"), "{latest}");
    assert!(harness.app.chat_widget.is_task_running());
    harness.key(KeyCode::Esc);
    harness.pump_until("the interruption", |harness| {
        harness.transcript().contains("Conversation interrupted")
    });
}

#[test]
fn esc_interrupts_a_side_conversations_turn_without_leaving_it() {
    let mut harness = Harness::timed(vec![
        now(assistant_events("Hi.")),
        slow(assistant_events("Too slow."), 3),
    ]);
    harness.answered("Hello.", "Hi.");
    harness.type_and_submit("/btw What is 2 + 2?");
    harness.pump_until("the side turn to run", |harness| {
        harness.app.workspace.second_focused() && harness.app.chat_widget.is_task_running()
    });
    harness.key(KeyCode::Esc);
    harness.pump_until("the side turn's interruption", |harness| {
        harness.transcript().contains("Conversation interrupted")
    });
    assert!(harness.app.workspace.second_focused());
}

#[test]
fn the_leaders_tab_is_the_chords_and_queues_nothing() {
    let mut harness = Harness::timed(vec![slow(assistant_events("Done."), 3)]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_draft("draft");
    harness.key_with(KeyCode::Char('x'), KeyModifiers::CONTROL);
    harness.key(KeyCode::Tab);
    harness.app.draw(&mut harness.tui).unwrap();
    assert_eq!(harness.app.chat_widget.composer_text(), "draft");
    assert!(!harness.screen().contains("Queued follow-up inputs"));
}

#[test]
fn a_subagent_shown_queues_its_own_messages_for_its_turns() {
    let mut harness = Harness::timed(vec![
        now(super::subagents::spawn_and_wait("Count the files.")),
        now(assistant_events("Twelve files.")),
        now(assistant_events("There are twelve.")),
        slow(assistant_events("Counting again."), 2),
        now(assistant_events("Steered count.")),
        now(assistant_events("First queued answer.")),
        now(assistant_events("Second queued answer.")),
    ]);
    let parent = harness.answered("Delegate the count.", "There are twelve.");
    let [child] = harness.subagents_of(parent)[..] else {
        panic!("one subagent");
    };
    harness.type_and_submit("/subagents");
    harness.pump_until_screen_shows("Enter shows one in place of this conversation");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Subagent counter");
    assert_eq!(harness.app.bridge.session_id(), Some(child));

    harness.type_and_submit("Count again.");
    harness.pump_until_running();
    harness.type_and_submit("Count the tests too.");
    harness.type_and_queue("Then once more.");
    harness.type_and_queue("And a last time.");
    harness.pump_until_screen_shows("Queued follow-up inputs");
    harness.pump_until("the last queued answer", |harness| {
        harness.transcript().contains("Second queued answer.")
    });
    harness.pump_until_turn_ends();
    harness.assert_in_order(&[
        "› Count again.",
        "Counting again.",
        "› Count the tests too.",
        "Steered count.",
        "› Then once more.",
        "First queued answer.",
        "› And a last time.",
        "Second queued answer.",
    ]);
    let transcript = harness.transcript();
    for text in [
        "Counting again.",
        "› Count the tests too.",
        "› Then once more.",
        "First queued answer.",
    ] {
        assert_eq!(
            transcript.matches(text).count(),
            1,
            "{text:?} in:\n{transcript}"
        );
    }
    assert_eq!(harness.server.requests().len(), 7);
    assert!(!harness.app.chat_widget.has_pending_input());
}
