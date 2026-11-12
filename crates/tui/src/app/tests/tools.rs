//! A tool call shows what its tool's presenter says, read off the app
//! loop's thread once it finished, live and replayed, in both
//! presentations: here `read_file`, its lines collapsed, its text numbered
//! expanded; and a command's output as it streams, before it finished.

use hanafubuki_agent::test_support::{assistant_events, tool_events};
use ratatui::crossterm::event::KeyCode;

use super::{command_call, has_output_row, Harness};
use crate::transcript_mode::TranscriptMode;

#[test]
fn a_running_commands_output_streams_into_its_entry_in_both_presentations() {
    // The command prints, then waits for the test to create `gate`.
    let command = if cfg!(windows) {
        "echo live-first; [Console]::Out.Flush(); while (-not (Test-Path gate)) { Start-Sleep -Milliseconds 100 }; echo live-second"
    } else {
        "echo live-first; while [ ! -e gate ]; do sleep 0.1; done; echo live-second"
    };
    for mode in [TranscriptMode::Owned, TranscriptMode::Terminal] {
        let mut harness = Harness::start_with(
            vec![command_call(command), assistant_events("The command ran.")],
            true,
            mode,
        );
        harness.type_and_submit("Run the command.");
        harness.pump_until_screen_shows("Yes, proceed");
        harness.key(KeyCode::Char('y'));
        harness.pump_until("the first line, streamed", |harness| {
            has_output_row(&harness.screen(), "live-first")
        });
        let screen = harness.screen();
        assert!(screen.contains("Running echo live-first"), "{screen}");
        assert!(!has_output_row(&screen, "live-second"), "{screen}");

        std::fs::write(harness.home.path().join("project").join("gate"), "").unwrap();
        harness.pump_until("the answer", |harness| {
            harness.screen().contains("The command ran.")
        });
        harness.pump_until("the finished command's output", |harness| {
            let screen = harness.screen();
            screen.contains("Ran echo live-first") && has_output_row(&screen, "live-second")
        });
        assert!(has_output_row(&harness.screen(), "live-first"));
    }
}

#[test]
fn a_read_shows_its_path_and_text_live_and_replayed() {
    for mode in [TranscriptMode::Owned, TranscriptMode::Terminal] {
        let arguments = serde_json::json!({ "path": "notes.txt", "offset": 0, "maxBytes": null });
        let mut harness = Harness::start_with(
            vec![
                tool_events("read_file", &arguments.to_string()),
                assistant_events("Read them."),
            ],
            true,
            mode,
        );
        std::fs::write(
            harness.home.path().join("project").join("notes.txt"),
            "first line\nsecond line\n",
        )
        .unwrap();

        let session_id = harness.exchange("Read the notes.", "Read them.");
        harness.pump_until("the read's lines", |harness| {
            harness
                .transcript()
                .contains("• Read notes.txt (lines 1–2)")
        });
        assert_numbered(&harness);

        harness.type_and_submit("/new");
        harness.pump_until("no session", |harness| {
            harness.app.bridge.session_id().is_none()
        });
        harness.type_and_submit(&format!("/resume {}", session_id.value()));
        harness.pump_until("the resumed transcript", |harness| {
            harness.transcript().contains("Read them.")
        });
        let transcript = harness.transcript();
        assert!(
            transcript.contains("• Read notes.txt (lines 1–2)"),
            "{transcript}"
        );
        assert_numbered(&harness);
    }
}

/// The read's text, numbered, is what its entry shows expanded.
fn assert_numbered(harness: &Harness) {
    let expanded: Vec<String> = harness
        .app
        .transcript_cells
        .iter()
        .flat_map(|cell| cell.expanded_lines(super::WIDTH))
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect();
    let at = expanded
        .iter()
        .position(|line| line.contains("1 first line"))
        .unwrap_or_else(|| panic!("{expanded:#?}"));
    assert_eq!(expanded[at + 1].trim(), "2 second line");
}
