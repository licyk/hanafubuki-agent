//! A tool call shows what its tool's presenter says, read off the app
//! loop's thread once it finished, live and replayed, in both
//! presentations: here `read_file`, whose structured output showed only
//! its header before.

use hanafubuki_agent::test_support::{assistant_events, tool_events};

use super::Harness;
use crate::transcript_mode::TranscriptMode;

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
        harness.pump_until("the read's text", |harness| {
            harness.transcript().contains("└ first line")
        });
        let transcript = harness.transcript();
        assert!(
            transcript.contains("Called read_file: notes.txt"),
            "{transcript}"
        );
        assert!(transcript.contains("second line"), "{transcript}");

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
            transcript.contains("Called read_file: notes.txt"),
            "{transcript}"
        );
        assert!(transcript.contains("└ first line"), "{transcript}");
    }
}
