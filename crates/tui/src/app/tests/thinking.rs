//! Thinking entries on an emulated terminal, in both presentations: each
//! collapsed under its control, a click on it or on its header showing or
//! hiding its text, and Ctrl-O every thinking at once, an approval open or
//! not, and in Scrollback's transcript.

use hanafubuki_agent::test_support::{
    late_reasoning_answer_events, reasoning_answer_events, reasoning_tool_events,
};

use super::*;

const MODES: [TranscriptMode; 2] = [TranscriptMode::Terminal, TranscriptMode::Owned];

/// The turn of `reasoning_answer_events`, ended, in `mode`.
fn thought(mode: TranscriptMode) -> Harness {
    let mut harness = Harness::start_with(
        vec![reasoning_answer_events(
            "Weighing the **question**.",
            "The answer is 42.",
        )],
        true,
        mode,
    );
    harness.type_and_submit("What is the answer?");
    harness.pump_until_screen_shows("The answer is 42.");
    harness.pump_until_turn_ends();
    harness
}

#[test]
fn a_click_on_a_thoughts_control_shows_and_hides_its_text() {
    for mode in MODES {
        let mut harness = thought(mode);
        let screen = harness.screen();
        assert!(screen.contains("• Thought · "), "{mode:?}:\n{screen}");
        assert!(!screen.contains("Weighing the question."), "{screen}");

        harness.click_on("+ Show details");
        harness.pump_until_screen_shows("Weighing the question.");
        harness.click_on("− Hide details");
        harness.app.draw(&mut harness.tui).unwrap();
        let screen = harness.screen();
        assert!(!screen.contains("Weighing the question."), "{screen}");
        assert!(screen.contains("+ Show details"), "{screen}");
    }
}

#[test]
fn a_click_on_a_thoughts_header_shows_and_hides_its_text() {
    for mode in MODES {
        let mut harness = thought(mode);
        harness.click_on("• Thought · ");
        harness.pump_until_screen_shows("Weighing the question.");
        harness.click_on("• Thought · ");
        harness.app.draw(&mut harness.tui).unwrap();
        let screen = harness.screen();
        assert!(
            !screen.contains("Weighing the question."),
            "{mode:?}:\n{screen}"
        );
    }
}

#[test]
fn the_transcript_takes_ctrl_o_and_a_click_on_a_header() {
    let mut harness = thought(TranscriptMode::Terminal);
    harness.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
    let pager = harness.pump_until_screen_shows("T R A N S C R I P T");
    assert!(!pager.contains("Weighing the question."), "{pager}");
    harness.key_with(KeyCode::Char('o'), KeyModifiers::CONTROL);
    harness.pump_until_screen_shows("Weighing the question.");
    harness.click_on("• Thought · ");
    harness.pump_until("the thought hidden", |harness| {
        !harness.screen().contains("Weighing the question.")
    });
    assert!(harness.screen().contains("T R A N S C R I P T"));
}

#[test]
fn ctrl_o_shows_every_thought_then_hides_them() {
    for mode in MODES {
        let mut harness = thought(mode);
        harness.key_with(KeyCode::Char('o'), KeyModifiers::CONTROL);
        let screen = harness.pump_until_screen_shows("Weighing the question.");
        assert!(screen.contains("− Hide details"), "{mode:?}:\n{screen}");
        harness.key_with(KeyCode::Char('o'), KeyModifiers::CONTROL);
        harness.pump_until("the thought hidden", |harness| {
            !harness.screen().contains("Weighing the question.")
        });
        assert!(harness.screen().contains("+ Show details"));
    }
}

#[test]
fn ctrl_o_reaches_the_thought_while_an_approval_waits() {
    for mode in MODES {
        let mut harness = Harness::start_with(
            vec![
                reasoning_tool_events(
                    "Run it first.",
                    "exec_command",
                    &command_arguments("echo thought-output"),
                ),
                reasoning_answer_events("It ran.", "The command ran."),
            ],
            true,
            mode,
        );
        harness.type_and_submit("Run the command.");
        harness.pump_until_screen_shows("Yes, proceed");
        harness.key_with(KeyCode::Char('o'), KeyModifiers::CONTROL);
        let screen = harness.pump_until_screen_shows("Run it first.");
        assert!(screen.contains("Yes, proceed"), "{mode:?}:\n{screen}");
        harness.key(KeyCode::Char('y'));
        harness.pump_until_screen_shows("The command ran.");
        harness.pump_until_turn_ends();
    }
}

/// The model thinks, starts its answer and only then completes the thought,
/// as the Responses API may: once the answer starts the entry has thought,
/// and the turn's thinking time, on the status line and the rule before the
/// answer, stops.
#[test]
fn the_thought_ends_when_the_answer_starts_before_its_late_completion() {
    for mode in MODES {
        // A streaming answer shows its complete lines.
        let mut harness = Harness::start_with(
            vec![
                tool_events("skills_list", "{}"),
                late_reasoning_answer_events(
                    "Weighing the question.",
                    "The answer is 42.\nThat is all.",
                ),
            ],
            true,
            mode,
        );
        harness.type_and_submit("What is the answer?");
        harness.pump_until_screen_shows("• Thinking");
        std::thread::sleep(Duration::from_millis(300));
        harness.server.hold.notify_one();

        let screen = harness.pump_until_screen_shows("The answer is 42.");
        assert!(screen.contains("• Thought · "), "{mode:?}:\n{screen}");
        assert!(!screen.contains("• Thinking"), "{mode:?}:\n{screen}");
        let thought = thinking_total(&screen);
        let transcript = harness.transcript();
        assert!(
            transcript.contains(&format!("· thought for {thought} ─")),
            "{mode:?}: {thought}\n{transcript}"
        );
        std::thread::sleep(Duration::from_millis(300));
        harness.app.draw(&mut harness.tui).unwrap();
        assert_eq!(thinking_total(&harness.screen()), thought, "{mode:?}");

        harness.server.hold.notify_one();
        harness.pump_until_turn_ends();
        let transcript = harness.transcript();
        assert_eq!(
            transcript.matches("• Thought · ").count(),
            1,
            "{mode:?}:\n{transcript}"
        );
    }
}

/// The time of the status line's `(Thinking 0.3s)`.
fn thinking_total(screen: &str) -> String {
    let start = screen.find("(Thinking ").expect(screen) + "(Thinking ".len();
    let length = screen[start..].find(')').expect(screen);
    screen[start..start + length].to_string()
}
