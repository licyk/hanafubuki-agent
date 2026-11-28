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

use hanafubuki_agent::test_support::{assistant_events, HOLD};
use hanafubuki_agent::{AgentSessionListRequest, AgentTurnPhase, ConversationItem};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::Harness;
use crate::agent_bridge::{AgentReply, SteerOutcome, TurnUpdate};
use crate::app_event::AppEvent;
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

fn held(events: Vec<u8>) -> (Vec<u8>, Duration) {
    ([HOLD.as_bytes(), &events].concat(), Duration::ZERO)
}

fn now(events: Vec<u8>) -> (Vec<u8>, Duration) {
    (events, Duration::ZERO)
}

impl Harness {
    fn timed(responses: Vec<(Vec<u8>, Duration)>) -> Self {
        Self::start_timed(responses, true, TranscriptMode::Terminal)
    }

    /// Types `text` into the composer without sending it.
    pub(super) fn type_draft(&mut self, text: &str) {
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

    /// Runs the app loop's steps until the Agent accepted a steer.
    pub(super) fn pump_until_steered(&mut self) {
        let deadline = Instant::now() + super::DEADLINE;
        loop {
            let event = self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("the Agent to accept the steering message");
            let accepted = matches!(&event,
                AppEvent::AgentReply(AgentReply::Update(update))
                    if matches!(update.as_ref(), TurnUpdate::Steered {
                        outcome: SteerOutcome::Accepted(_), ..
                    })
            );
            assert!(self.app.handle_event(&mut self.tui, event));
            if accepted {
                return;
            }
        }
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
fn async_questions_keep_working_and_deliver_only_the_chosen_answer() {
    let questions = hanafubuki_agent::test_support::tool_events(
        "request_user_input_async",
        r#"{"questions":[{"title":"Which scope?","options":["Whole workspace","Tests only"]},{"title":"What else should I know?"}]}"#,
    );
    let mut harness = Harness::timed(vec![
        now(questions),
        held(assistant_events("Independent work.")),
        now(assistant_events("Received.")),
    ]);
    harness.type_and_submit("Start.");
    harness.pump_until_screen_shows("2 questions");
    harness.pump_until("sampling to continue before any answer", |h| {
        h.server.requests().len() == 2
    });
    harness.type_draft("Main draft.");
    harness.key_with(KeyCode::Left, KeyModifiers::SHIFT);
    harness.pump_until_screen_shows("Question 1 of 2");
    harness.key_with(KeyCode::Left, KeyModifiers::SHIFT);
    harness.pump_until_screen_shows("Question 2 of 2");
    assert!(harness
        .app
        .handle_event(&mut harness.tui, AppEvent::Paste("待补充".into())));
    harness.key(KeyCode::BackTab);
    harness.pump_until_screen_shows("Question 1 of 2");
    harness.key(KeyCode::Down);
    harness.pump_until_screen_shows("› 2. Tests only");
    harness.key(KeyCode::Enter);
    harness.pump_until_steered();
    harness.key(KeyCode::Esc);
    assert_eq!(harness.app.chat_widget.composer_text(), "Main draft.");
    harness.server.hold.notify_one();
    harness.pump_until_screen_shows("Received.");
    harness.pump_until_turn_ends();
    let prompts = harness.saved_prompts();
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    let replies = hanafubuki_agent::AgentAsyncQuestionReply::from_message(
        prompts[1].strip_prefix("steering: ").unwrap(),
    );
    assert_eq!(
        replies[0].question_item_id,
        hanafubuki_agent::AgentAsyncQuestion::item_id("call-tool", 0)
    );
    assert_eq!(replies[0].answer, "Tests only");
    assert_eq!(replies[0].question, "Which scope?");
    let draft = harness.app.chat_widget.composer_text();
    assert!(
        draft.starts_with("Main draft.") && draft.ends_with("待补充"),
        "{draft}"
    );
    assert!(harness.transcript().contains("Which scope? → Tests only"));
}

#[test]
fn enter_steers_the_running_turn_which_reads_it_at_its_next_step() {
    let mut harness = Harness::timed(vec![
        held(assistant_events("Working on it.")),
        now(assistant_events("Steered.")),
    ]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_submit("Also check the tests.");
    harness.pump_until_steered();
    let waiting = harness.pump_until_screen_shows("Messages to be submitted after next tool call");
    assert!(waiting.contains("↳ Also check the tests."), "{waiting}");
    // Not written until the turn reads it.
    assert!(!harness.transcript().contains("Also check the tests."));

    harness.server.hold.notify_one();
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
fn consumed_inputs_return_the_working_line_to_the_composer() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        for queued in [false, true] {
            let held_answer = [HOLD.as_bytes(), &assistant_events("Continue.")].concat();
            let mut harness =
                Harness::start_with(vec![held_answer.clone(), held_answer], true, mode);
            harness.type_and_submit("Start.");
            harness.pump_until_running();
            harness.pump_until("the first model request to be held", |harness| {
                harness.server.requests().len() == 1
            });
            let gap = working_composer_gap(&harness.screen());
            let message = "Also check the tests before continuing with the documentation.";
            let header = if queued {
                harness.type_and_queue(message);
                "Queued follow-up inputs"
            } else {
                harness.type_and_submit(message);
                "Messages to be submitted after next tool call"
            };
            let waiting = harness.pump_until_screen_shows(header);
            assert!(working_composer_gap(&waiting) > gap, "{mode:?}:\n{waiting}");

            harness.server.hold.notify_one();
            harness.pump_until("the input consumed and the model continuing", |harness| {
                harness.transcript().contains("› Also check the tests")
                    && !harness.screen().contains(header)
                    && harness.app.chat_widget.is_task_running()
                    && harness.server.requests().len() == 2
            });
            assert!(harness.app.chat_widget.is_task_running());
            let screen = harness.screen();
            assert_eq!(working_composer_gap(&screen), gap, "{mode:?}:\n{screen}");

            harness.server.hold.notify_one();
            harness.pump_until_turn_ends();
        }
    }
}

#[test]
fn editing_queued_inputs_releases_the_preview_rows() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let held_answer = [HOLD.as_bytes(), &assistant_events("Done.")].concat();
        let mut harness = Harness::start_with(vec![held_answer], true, mode);
        harness.type_and_submit("Start.");
        harness.pump_until_running();
        let gap = working_composer_gap(&harness.screen());
        harness.type_and_queue("First queued message.");
        let one = harness.pump_until_screen_shows("Queued follow-up inputs");
        let one_gap = working_composer_gap(&one);
        harness.type_and_queue("Second queued message.");
        let two = harness.pump_until_screen_shows("↳ Second queued message.");
        assert!(working_composer_gap(&two) > one_gap, "{mode:?}:\n{two}");

        harness.key_with(KeyCode::Left, KeyModifiers::SHIFT);
        harness.app.draw(&mut harness.tui).unwrap();
        let screen = harness.screen();
        assert_eq!(
            working_composer_gap(&screen),
            one_gap,
            "{mode:?}:\n{screen}"
        );
        harness.key_with(KeyCode::Left, KeyModifiers::SHIFT);
        harness.app.draw(&mut harness.tui).unwrap();
        let screen = harness.screen();
        assert!(!screen.contains("Queued follow-up inputs"), "{screen}");
        assert_eq!(working_composer_gap(&screen), gap, "{mode:?}:\n{screen}");

        harness.server.hold.notify_one();
        harness.pump_until_turn_ends();
    }
}

fn working_composer_gap(screen: &str) -> usize {
    let lines: Vec<_> = screen.lines().collect();
    let working = lines
        .iter()
        .position(|line| line.contains("esc to interrupt"))
        .expect(screen);
    let composer = lines
        .iter()
        .rposition(|line| line.starts_with('›'))
        .expect(screen);
    composer - working
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
        // The server answers one request at a time: "Resent." waits for
        // this one.
        held(assistant_events("Too slow.")),
        now(assistant_events("Resent.")),
    ]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_submit("First steer.");
    harness.type_and_submit("Second steer.");
    harness.pump_until_screen_shows("↳ Second steer.");
    harness.type_draft("a draft");

    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Model interrupted to submit steer instructions.");
    harness.server.hold.notify_one();
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
        held(assistant_events("First.")),
        now(assistant_events("Second.")),
        now(assistant_events("Third.")),
    ]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_queue("Queued one.");
    harness.type_and_queue("Queued two.");
    let queued = harness.pump_until_screen_shows("Queued follow-up inputs");
    assert!(queued.contains("↳ Queued two."), "{queued}");

    harness.server.hold.notify_one();
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
        held(failing_events()),
        now(assistant_events("Recovered.")),
    ]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_queue("After the failure.");
    harness.server.hold.notify_one();
    harness.pump_until_screen_shows("Recovered.");
    harness.assert_in_order(&["■", "› After the failure.", "Recovered."]);
}

#[test]
fn an_interrupt_puts_the_queue_back_in_the_composer_and_sends_nothing() {
    let mut harness = Harness::timed(vec![held(assistant_events("Too slow."))]);
    harness.type_and_submit("Start.");
    harness.pump_until_running();
    harness.type_and_queue("Queued.");
    harness.pump_until_screen_shows("Queued follow-up inputs");
    harness.pump_until("the first request to reach the server", |harness| {
        harness.server.requests().len() == 1
    });
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
        vec![held(assistant_events("Done."))],
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
        held(assistant_events("Main done.")),
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
    harness.server.hold.notify_one();
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
fn submitting_a_message_returns_to_latest_but_editing_does_not() {
    for (running, submit_key) in [
        (false, KeyCode::Enter),
        (true, KeyCode::Enter),
        (true, KeyCode::Tab),
    ] {
        let mut harness = Harness::start_timed(
            vec![held(assistant_events("Too slow."))],
            true,
            TranscriptMode::Owned,
        );
        if running {
            harness.type_and_submit("Start.");
            harness.pump_until_running();
        }
        for note in 0..60 {
            harness
                .app
                .chat_widget
                .add_info_message(format!("note {note:02}"));
        }
        harness.pump_until_screen_shows("note 59");
        harness.key(KeyCode::PageUp);
        harness.type_draft("Continue.");
        harness.key_with(KeyCode::Char('j'), KeyModifiers::CONTROL);
        harness.app.draw(&mut harness.tui).unwrap();
        assert!(!harness.app.transcript_view.is_following());
        assert!(harness.screen().contains("Back to bottom"));

        harness.key(submit_key);
        harness.pump_until("the submitted message to resume following", |harness| {
            harness.app.transcript_view.is_following()
                && !harness.screen().contains("Back to bottom")
        });
        assert!(harness.screen().contains("note 59"));
        assert!(harness.app.chat_widget.composer_is_empty());
    }
}

#[test]
fn esc_returns_a_scrolled_conversation_to_the_bottom_before_it_interrupts() {
    let mut harness = Harness::start_timed(
        vec![held(assistant_events("Too slow."))],
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
        held(assistant_events("Too slow.")),
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
    let mut harness = Harness::timed(vec![held(assistant_events("Done."))]);
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
        held(assistant_events("Counting again.")),
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
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Subagent counter");
    assert_eq!(harness.app.bridge.session_id(), Some(child));

    harness.type_and_submit("Count again.");
    harness.pump_until_running();
    harness.type_and_submit("Count the tests too.");
    harness.type_and_queue("Then once more.");
    harness.type_and_queue("And a last time.");
    harness.pump_until_screen_shows("Queued follow-up inputs");
    harness.server.hold.notify_one();
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
