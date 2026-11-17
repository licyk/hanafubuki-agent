//! A command's output in its colours: the same spans while it streams and
//! once its display arrives, dimmed when collapsed, kept across wrapping
//! when expanded, and none of them in the raw text.

use hanafubuki_agent::{
    render_styled_text, AgentExecutionProgressStatus, AgentTextStyle, AgentToolCallDisplay,
};
use ratatui::style::{Color, Modifier, Style};

use super::*;

/// What the process printed, in pieces as the stream brings them.
const PRINTED: [&str; 3] = [
    "\x1b[1;32m   Compiling\x1b[0m foo v0.1.0\n\x1b[33mwarning",
    "\x1b[0m: unused\tvariable\n\x1b[31mred across\n",
    "two lines\x1b[0m done\n",
];

/// The display the execution service gives that output: its plain text
/// with the styles beside it.
fn styled_command() -> AgentToolDisplay {
    let rendered = render_styled_text(PRINTED.concat().as_bytes(), AgentTextStyle::default());
    let AgentToolDisplay::Command {
        command,
        cwd,
        exit_code,
        duration_millis,
        truncated,
        ..
    } = command("cargo build", "", Some(0))
    else {
        unreachable!()
    };
    AgentToolDisplay::Command {
        command,
        cwd,
        output: rendered.text,
        exit_code,
        duration_millis,
        truncated,
        styled: Some(rendered.styles),
    }
}

fn streamed() -> ToolCallCell {
    let mut cell = new_tool_call(
        "call-1".to_string(),
        "exec_command".to_string(),
        Some("cargo build".to_string()),
    );
    for piece in PRINTED {
        cell.push_output("call-1", piece, false);
    }
    cell
}

/// The output rows of `lines` (under the header), with their spans.
fn output_spans(lines: &[Line<'static>]) -> Vec<Vec<(String, Style)>> {
    lines
        .iter()
        .skip(1)
        .filter(|line| !text(std::slice::from_ref(line)).contains('✓'))
        .filter(|line| !text(std::slice::from_ref(line)).contains("$ cargo"))
        .map(|line| {
            line.spans
                .iter()
                .skip(1)
                .map(|span| (span.content.to_string(), line.style.patch(span.style)))
                .collect()
        })
        .collect()
}

fn green_bold() -> Style {
    Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD)
}

#[test]
fn command_syntax_colours_are_present_from_start_through_completion() {
    let script = "printf '%s\\n' \"$HOME\" && echo done";
    for (by_user, background) in [(false, false), (true, false), (false, true)] {
        let mut cell = new_tool_call("call-1".into(), "exec_command".into(), Some(script.into()));
        if by_user {
            cell = cell.typed_by_user();
        }
        // Compare the command's styled characters, excluding the bullet,
        // lifecycle verb and running clock.
        let colours = |cell: &ToolCallCell| {
            let lines = cell.display_lines(120);
            let header = &lines[0];
            let shown = text(std::slice::from_ref(header));
            let start = shown[..shown.find(script).unwrap()].chars().count();
            header
                .spans
                .iter()
                .flat_map(|span| {
                    span.content
                        .chars()
                        .map(move |character| (character, header.style.patch(span.style)))
                })
                .skip(start)
                .take(script.chars().count())
                .collect::<Vec<_>>()
        };
        let started = colours(&cell);
        assert!(
            started.iter().any(|(_, style)| style.fg.is_some()),
            "command is unhighlighted: {started:?}"
        );
        cell.push_output("call-1", "working\n", false);
        assert_eq!(colours(&cell), started);
        if background {
            cell.execution_status("call-1", AgentExecutionProgressStatus::Running, None);
        }
        cell.complete("call-1", false);
        assert_eq!(colours(&cell), started);
        if background {
            cell.show(
                "call-1",
                AgentToolCallDisplay {
                    subject: None,
                    display: Some(command(script, "working\n", None)),
                },
            );
            assert_eq!(colours(&cell), started);
            cell.execution_status("call-1", AgentExecutionProgressStatus::Exited, Some(0));
            assert_eq!(colours(&cell), started);
        }
        cell.show(
            "call-1",
            AgentToolCallDisplay {
                subject: None,
                display: Some(command(script, "done\n", Some(0))),
            },
        );
        assert_eq!(colours(&cell), started);
    }
}

#[test]
fn a_multiline_command_is_highlighted_in_details_before_any_output() {
    let script = "echo 'first'\nprintf '%s' \"$HOME\"";
    let mut cell = new_tool_call("call-1".into(), "exec_command".into(), Some(script.into()));
    let running = cell.transcript_lines(120);
    let shown = text(&running);
    assert!(shown.contains("Running echo 'first' …"), "{shown}");
    assert!(shown.contains("$ echo 'first'"), "{shown}");
    assert!(shown.contains("printf '%s' \"$HOME\""), "{shown}");
    cell.complete("call-1", false);
    cell.show(
        "call-1",
        AgentToolCallDisplay {
            subject: None,
            display: Some(command(script, "", Some(0))),
        },
    );
    let finished = cell.transcript_lines(120);
    assert_eq!(running[1..3], finished[1..3]);
    assert!(running[1..3]
        .iter()
        .flat_map(|line| &line.spans)
        .any(|span| span.style.fg.is_some()));
}

#[test]
fn live_and_finished_output_show_the_same_colours_collapsed_and_expanded() {
    let mut cell = streamed();
    let live_collapsed = output_spans(&cell.display_lines(80));
    let live_expanded = output_spans(&cell.transcript_lines(80));
    cell.complete("call-1", false);
    cell.show(
        "call-1",
        AgentToolCallDisplay {
            subject: None,
            display: Some(styled_command()),
        },
    );
    assert_eq!(output_spans(&cell.display_lines(80)), live_collapsed);
    assert_eq!(output_spans(&cell.transcript_lines(80)), live_expanded);

    let red = Style::default().fg(Color::Red);
    assert_eq!(
        live_expanded,
        [
            vec![
                ("   Compiling".to_string(), green_bold()),
                (" foo v0.1.0".to_string(), Style::default()),
            ],
            vec![
                ("warning".to_string(), Style::default().fg(Color::Yellow)),
                (": unused variable".to_string(), Style::default()),
            ],
            vec![("red across".to_string(), red)],
            vec![
                ("two lines".to_string(), red),
                (" done".to_string(), Style::default()),
            ],
        ]
    );
    // Collapsed, the last rows keep their colours, dimmed.
    let dim = Style::default().add_modifier(Modifier::DIM);
    assert_eq!(
        live_collapsed[2],
        vec![
            ("two lines".to_string(), dim.patch(red)),
            (" done".to_string(), dim),
        ]
    );
}

#[test]
fn a_wrapped_row_keeps_its_colours_and_the_raw_text_has_none() {
    let cell = shown("exec_command", Some("cargo build"), styled_command());
    let rows = output_spans(&cell.transcript_lines(16));
    // `   Compiling foo v0.1.0` wraps at 12 columns: the colour stays with
    // its word.
    let compiling = rows
        .iter()
        .position(|row| row.iter().any(|(text, _)| text.contains("Compiling")))
        .unwrap();
    assert_eq!(
        rows[compiling],
        vec![("   Compiling".to_string(), green_bold())]
    );
    assert_eq!(
        rows[compiling + 1],
        vec![("foo v0.1.0".to_string(), Style::default())]
    );
    for line in cell.raw_lines() {
        for span in &line.spans {
            assert_eq!(span.style, Style::default(), "{line:?}");
            assert!(!span.content.contains('\u{1b}'));
        }
    }
}

#[test]
fn a_command_the_person_typed_shows_its_colours() {
    let cell = new_user_shell_command(
        "user-shell-1".to_string(),
        AgentToolCallDisplay {
            subject: Some("cargo build".to_string()),
            display: Some(styled_command()),
        },
    );
    let lines = cell.transcript_lines(80);
    assert!(
        text(&lines).contains("You ran cargo build"),
        "{}",
        text(&lines)
    );
    assert!(output_spans(&lines)
        .iter()
        .flatten()
        .any(|(text, style)| text == "   Compiling" && *style == green_bold()));
}

#[test]
fn a_display_without_styles_is_drawn_as_before() {
    let plain = shown(
        "exec_command",
        Some("cargo build"),
        command("cargo build", "   Compiling foo\nwarning: x\n", Some(0)),
    );
    let rows = output_spans(&plain.transcript_lines(80));
    assert_eq!(
        rows,
        [
            vec![("   Compiling foo".to_string(), Style::default())],
            vec![("warning: x".to_string(), Style::default())],
        ]
    );
}
