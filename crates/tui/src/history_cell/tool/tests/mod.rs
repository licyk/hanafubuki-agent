//! The tool call entry: each display's view collapsed and expanded, what
//! expanding adds, the frame while a call runs, and the exploration run.

mod exploration;
mod views;

use hanafubuki_agent::{AgentToolCallDisplay, AgentToolDisplay};
use ratatui::text::Line;

use super::*;

fn text(lines: &[Line<'static>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render(cell: &ToolCallCell, width: u16) -> String {
    text(&cell.display_lines(width))
}

/// The entry collapsed, then expanded, as a snapshot reads them.
fn both(cell: &ToolCallCell, width: u16) -> String {
    format!(
        "{}\n─── expanded ({:?}) ───\n{}",
        render(cell, width),
        cell.activity_disclosure(width),
        text(&cell.transcript_lines(width))
    )
}

/// A `name` call that finished showing `display`.
fn shown(name: &str, subject: Option<&str>, display: AgentToolDisplay) -> ToolCallCell {
    let mut cell = new_tool_call(
        "call-1".to_string(),
        name.to_string(),
        subject.map(str::to_string),
    );
    cell.finish("call-1", display);
    cell
}

fn command(command: &str, output: &str, exit_code: Option<i32>) -> AgentToolDisplay {
    AgentToolDisplay::Command {
        command: command.to_string(),
        cwd: None,
        output: output.to_string(),
        exit_code,
        duration_millis: Some(4_200),
        truncated: false,
    }
}

fn numbered(lines: usize) -> String {
    (1..=lines).map(|n| format!("line {n}\n")).collect()
}

#[test]
fn a_running_call_shows_its_command_or_its_name_and_subject() {
    let cell = new_tool_call(
        "call-1".to_string(),
        "exec_command".to_string(),
        Some("cargo test\u{1b}[2J".to_string()),
    );
    assert!(
        render(&cell, 60).ends_with("Running cargo test"),
        "{}",
        render(&cell, 60)
    );
    assert_eq!(cell.activity_disclosure(60), None);
    assert!(cell.transcript_animation_tick().is_some());
    let cell = new_tool_call(
        "call-1".to_string(),
        "web_search".to_string(),
        Some("rust".to_string()),
    );
    assert!(render(&cell, 60).ends_with("Running web_search: rust"));
}

#[test]
fn a_finished_call_without_a_display_shows_its_name() {
    let mut cell = new_tool_call(
        "call-1".to_string(),
        "web_search".to_string(),
        Some("rust".to_string()),
    );
    cell.complete("call-1", true);
    assert_eq!(render(&cell, 60), "• Failed web_search: rust");
    // A command keeps its header from when it ran.
    let mut cell = new_tool_call(
        "call-1".to_string(),
        "exec_command".to_string(),
        Some("cargo test".to_string()),
    );
    cell.complete("call-1", true);
    assert_eq!(render(&cell, 60), "• Ran cargo test");
    assert_eq!(cell.activity_disclosure(60), None);
    assert_eq!(cell.transcript_animation_tick(), None);
}

#[test]
fn a_command_shows_its_last_lines_and_expands_to_its_whole_output() {
    let short = shown(
        "exec_command",
        Some("echo hi"),
        command("echo hi", "hi\n", Some(0)),
    );
    assert_eq!(render(&short, 60), "• Ran echo hi\n  └ hi");
    // Nothing more to show: no control.
    assert_eq!(short.activity_disclosure(60), None);

    let long = shown(
        "exec_command",
        Some("cargo test"),
        command("cargo test", &numbered(14), Some(101)),
    );
    assert_eq!(
        long.activity_disclosure(60),
        Some(ActivityDisclosure::OutputLines(11))
    );
    assert_eq!(long.activity_ids(), vec!["tool:call-1".to_string()]);

    let silent = shown("exec_command", Some("true"), command("true", "", Some(0)));
    assert_eq!(render(&silent, 60), "• Ran true\n  └ (no output)");
}

#[test]
fn a_session_written_to_or_waited_for_names_the_session() {
    let poll = shown("write_stdin", Some("session 3"), command("", "", None));
    assert_eq!(render(&poll, 60), "• Waited for session 3");
    let typed = shown(
        "write_stdin",
        Some("session 3 ← ls\\n"),
        command("ls\\n", "a.txt\n", None),
    );
    assert_eq!(
        render(&typed, 60),
        "• Interacted with session 3 ← ls\\n\n  └ a.txt"
    );
}

#[test]
fn a_compress_call_folds_its_summaries_and_receipt() {
    let mut cell = new_tool_call("fold-1".to_string(), "compress".to_string(), None);
    cell.complete("fold-1", false);
    // The kernel's presenter: the ranges in the subject, the receipt then
    // each range's summary in the text.
    cell.show("fold-1", AgentToolCallDisplay {
        subject: Some("m00002–m00004 Setup, m00006–m00007".to_string()),
        display: Some(AgentToolDisplay::Text {
            text: "[Compressed m00002–m00004 → 1 block(s), ~8000 tokens saved.]\n · b1 full summary 120ch · head \"The setup\" … tail \"done.\"\nFingerprints show head/tail only.\nm00002–m00004 Setup\nThe setup ran.\nEverything passed.\nm00006–m00007\nRead the logs."
                .to_string(),
        }),
    });

    assert_eq!(
        render(&cell, 100).replace('\u{2022}', "*"),
        "* Called compress: m00002–m00004 Setup, m00006–m00007\n  └ [Compressed m00002–m00004 → 1 block(s), ~8000 tokens saved.]"
    );
    assert_eq!(
        cell.activity_disclosure(100),
        Some(ActivityDisclosure::Generic)
    );
    let detailed = text(&cell.transcript_lines(100));
    let detailed: Vec<&str> = detailed.lines().collect();
    assert_eq!(
        detailed[detailed.len() - 5..],
        [
            "    m00002–m00004 Setup",
            "    The setup ran.",
            "    Everything passed.",
            "    m00006–m00007",
            "    Read the logs.",
        ]
    );
    assert!(detailed.iter().any(|line| line.contains("Fingerprints")));
}

#[test]
fn a_display_without_a_subject_keeps_the_one_the_call_started_with() {
    let mut cell = new_tool_call(
        "call-1".to_string(),
        "mcp__docs__lookup".to_string(),
        Some("rust".to_string()),
    );
    cell.complete("call-1", false);
    cell.show(
        "call-1",
        AgentToolCallDisplay {
            subject: None,
            display: Some(AgentToolDisplay::Text {
                text: "ok".to_string(),
            }),
        },
    );
    assert_eq!(
        render(&cell, 60),
        "• Called mcp__docs__lookup: rust\n  └ ok"
    );
    // A display for another call changes nothing.
    cell.show(
        "call-2",
        AgentToolCallDisplay {
            subject: Some("other".to_string()),
            display: None,
        },
    );
    assert!(render(&cell, 60).contains(": rust"));
}

#[test]
fn a_long_preview_row_is_cut_and_its_whole_line_is_a_detail() {
    let cell = shown(
        "mcp__docs__lookup",
        None,
        AgentToolDisplay::Text {
            text: "a line much wider than the entry it is shown in".to_string(),
        },
    );
    assert_eq!(
        render(&cell, 24),
        "• Called\n    mcp__docs__lookup\n  └ a line much wider t…"
    );
    assert_eq!(
        cell.activity_disclosure(24),
        Some(ActivityDisclosure::Generic)
    );
    assert!(text(&cell.transcript_lines(24)).contains("shown in"));
}

#[test]
fn a_long_command_line_is_cut_in_the_header() {
    let line = format!("echo {}", "x".repeat(400));
    let cell = shown("exec_command", Some("echo"), command(&line, "x\n", Some(0)));
    let header: String = render(&cell, u16::MAX).lines().next().unwrap().to_string();
    // `• Ran ` and 200 columns, the last one the ellipsis.
    assert_eq!(header.chars().count(), 6 + 200);
    assert!(header.ends_with('…'), "{header}");
    assert_eq!(
        cell.activity_disclosure(80),
        Some(ActivityDisclosure::Generic)
    );
}

#[test]
fn a_tab_is_measured_as_the_transcript_lays_it_out() {
    let cell = shown(
        "exec_command",
        Some("git status"),
        command(
            "git status",
            "\tmodified:   a-file-with-a-long-name.rs\n",
            Some(0),
        ),
    );
    // Expanded to the next stop, then cut to the row.
    assert_eq!(
        render(&cell, 30),
        "• Ran git status\n  └     modified:   a-file-wi…"
    );
    let read = shown(
        "read_file",
        Some("Makefile"),
        AgentToolDisplay::FileRead {
            path: "Makefile".to_string(),
            start_line: Some(1),
            text: "all:\n\tcc -o main main.c\n".to_string(),
            truncated: false,
        },
    );
    assert_eq!(
        text(&read.transcript_lines(60)),
        "• Read Makefile (lines 1–2)\n  └ 1 all:\n    2     cc -o main main.c"
    );
}

#[test]
fn a_failed_call_with_a_display_has_a_red_bullet() {
    let mut cell = new_tool_call(
        "call-1".to_string(),
        "list_directory".to_string(),
        Some("src".to_string()),
    );
    cell.complete("call-1", true);
    cell.show(
        "call-1",
        AgentToolCallDisplay {
            subject: None,
            display: Some(AgentToolDisplay::Listing {
                path: "src".to_string(),
                entries: Vec::new(),
                truncated: false,
            }),
        },
    );
    let header = cell.display_lines(60).remove(0);
    assert_eq!(header.spans[0].style.fg, Some(ratatui::style::Color::Red));
}
