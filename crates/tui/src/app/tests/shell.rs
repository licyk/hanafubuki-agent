//! `!command`s end to end over the standalone Agent: the command runs as a
//! turn that asks no model and shows as `You ran`, the next message's
//! request carries it, and a resumed session replays it; `!` alone says
//! how; a turn running refuses one and keeps it in the composer; a message
//! sent while one runs waits for it; Esc stops it. The commands are ones
//! every platform's default shell runs (`echo`, `sleep`: PowerShell on
//! Windows).

use std::time::Duration;

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::KeyCode;

use super::{has_output_row, Harness};
use crate::transcript_mode::TranscriptMode;

impl Harness {
    /// The bodies of the requests the model was sent, oldest first.
    fn request_bodies(&self) -> Vec<String> {
        self.server
            .requests()
            .iter()
            .map(|request| String::from_utf8_lossy(&request.body).into_owned())
            .collect()
    }

    fn type_without_sending(&mut self, text: &str) {
        for character in text.chars() {
            self.key(KeyCode::Char(character));
        }
        std::thread::sleep(Duration::from_millis(100));
        self.app.draw(&mut self.tui).unwrap();
    }
}

#[test]
fn a_shell_command_runs_without_the_model_and_the_next_message_carries_it() {
    let mut harness = Harness::start(vec![assistant_events("It printed the marker.")]);
    harness.type_without_sending("!echo tui-shell-marker");
    harness.pump_until_screen_shows("! shell command · runs in the session's sandbox");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("You ran echo tui-shell-marker");
    harness.pump_until_turn_ends();
    harness.pump_until("its output", |harness| {
        has_output_row(&harness.screen(), "tui-shell-marker")
    });
    assert!(harness.request_bodies().is_empty(), "no model was asked");
    // The composer's history has it as typed.
    harness.key(KeyCode::Up);
    harness.pump_until_screen_shows("› !echo tui-shell-marker");
    harness.key(KeyCode::Down);

    harness.type_and_submit("What did it print?");
    harness.pump_until_screen_shows("It printed the marker.");
    harness.pump_until_turn_ends();
    let bodies = harness.request_bodies();
    assert_eq!(bodies.len(), 1);
    assert!(
        bodies[0].contains("<user_shell_command>\\n<command>\\necho tui-shell-marker"),
        "{}",
        bodies[0]
    );

    harness.type_and_submit("/new");
    harness.pump_until("no session", |harness| {
        harness.app.bridge.session_id().is_none()
    });
    harness.type_and_submit("/resume");
    harness.pump_until_screen_shows("Resume a previous session");
    harness.key(KeyCode::Enter);
    harness.pump_until("the resumed transcript", |harness| {
        harness.transcript().contains("It printed the marker.")
    });
    let transcript = harness.transcript();
    assert!(
        transcript.contains("• You ran echo tui-shell-marker"),
        "{transcript}"
    );
    assert!(
        has_output_row(&transcript, "tui-shell-marker"),
        "{transcript}"
    );
}

#[test]
fn a_bare_bang_says_how_and_a_running_turn_refuses_a_command() {
    let mut harness = Harness::start_timed(
        vec![(assistant_events("Slow answer."), Duration::from_secs(60))],
        true,
        TranscriptMode::Terminal,
    );
    harness.type_and_submit("!");
    harness.pump_until_screen_shows("Prefix a command with ! to run it locally");

    harness.type_and_submit("Take your time.");
    harness.pump_until("the turn to run", |harness| {
        harness.app.chat_widget.is_task_running()
    });
    harness.type_and_submit("!echo too-early");
    harness.pump_until_screen_shows("A turn is running; wait for it or interrupt it first.");
    harness.pump_until_screen_shows("› !echo too-early");
    assert_eq!(harness.request_bodies().len(), 1);
}

#[test]
fn a_message_waits_for_the_command_and_esc_stops_one() {
    let mut harness = Harness::start(vec![assistant_events("After the command.")]);
    harness.type_and_submit("!sleep 2");
    harness.pump_until("the command to run", |harness| {
        harness.app.chat_widget.is_task_running()
    });
    // It reads no messages: this one waits, then starts its own turn.
    harness.type_and_submit("And now?");
    harness.pump_until_screen_shows("After the command.");
    harness.pump_until_turn_ends();
    let transcript = harness.transcript();
    assert!(transcript.contains("You ran sleep 2"), "{transcript}");
    let bodies = harness.request_bodies();
    assert!(bodies[0].contains("<user_shell_command>"), "{}", bodies[0]);

    // Longer than the harness's wait: only Esc ends it in time.
    harness.type_and_submit("!sleep 120");
    harness.pump_until_screen_shows("Running sleep 120");
    harness.key(KeyCode::Esc);
    harness.pump_until_turn_ends();
    let transcript = harness.transcript();
    assert!(transcript.contains("You ran sleep 120"), "{transcript}");
    assert!(
        !transcript.contains("tell the model what to do differently"),
        "{transcript}"
    );
}
