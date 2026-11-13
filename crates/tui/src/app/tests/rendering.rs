//! A model's answer with a table, a task list and links, end to end over
//! the standalone Agent in both presentations; and `tui.rendering.tables`
//! turned off in the settings center, the next answer's table shown as the
//! model wrote it.

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::KeyCode;

use super::Harness;
use crate::transcript_mode::TranscriptMode;

const ANSWER: &str = "Results:\n\n| Case | Result |\n|:---|---:|\n| first | passed |\n| second | failed |\n\n- [x] Built\n- [ ] Released\n\nSee [the docs](https://example.com/docs) and [the parser](/project/src/parse.rs#L12C3).\n";

/// The screen's lines, their trailing blanks left out.
fn lines(screen: &str) -> Vec<String> {
    screen
        .lines()
        .map(|line| line.trim_end().to_string())
        .collect()
}

#[test]
fn an_answers_table_task_list_and_links_render_in_both_presentations() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(vec![assistant_events(ANSWER)], true, mode);
        harness.type_and_submit("Report");
        let screen = lines(&harness.pump_until_screen_shows("(src/parse.rs:12:3)"));
        for expected in [
            "   Case      Result",
            "  ━━━━━━━━  ━━━━━━━━",
            "   first     passed",
            "  ────────  ────────",
            "   second    failed",
            "  ☑ Built",
            "  ☐ Released",
            "  See the docs (https://example.com/docs) and the parser (src/parse.rs:12:3).",
        ] {
            assert!(
                screen.iter().any(|line| line == expected),
                "{mode:?}: no line {expected:?} in\n{}",
                screen.join("\n")
            );
        }
    }
}

#[test]
fn tables_turned_off_show_the_next_answers_table_as_written() {
    let mut harness = Harness::owned(vec![assistant_events(ANSWER)]);
    harness.type_and_submit("/config interface");
    harness.pump_until_screen_shows("Terminal interface");
    harness.select_setting("Tables");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved");
    assert!(harness.config_file().contains("tables = false"));
    harness.key(KeyCode::Esc);
    harness.pump_until("the center to close", |harness| {
        !harness.screen().contains("Changes save automatically")
            && !harness.screen().contains("Saved")
    });
    harness.type_and_submit("Report");
    let screen = lines(&harness.pump_until_screen_shows("| second | failed |"));
    assert!(
        screen.iter().any(|line| line == "  |:---|---:|"),
        "{}",
        screen.join("\n")
    );
    assert!(screen.iter().any(|line| line == "  ☑ Built"));
}
