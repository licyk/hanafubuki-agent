//! The live buffer: lines as a terminal shows them, redraws in place, and
//! its bounds.

use ratatui::style::{Color, Modifier, Style};

use super::super::styled_output::line_text;
use super::*;

fn lines(output: &LiveOutput) -> Vec<String> {
    plain(output).0
}

/// The lines' text, and how many lines they stand for.
fn plain(output: &LiveOutput) -> (Vec<String>, usize) {
    let (lines, count) = output.lines();
    (lines.iter().map(line_text).collect(), count)
}

fn pushed(chunks: &[&str]) -> LiveOutput {
    let mut output = LiveOutput::default();
    for chunk in chunks {
        output.push(chunk, false);
    }
    output
}

#[test]
fn ended_lines_are_committed_and_the_unfinished_one_shows_as_it_is_now() {
    let output = pushed(&["first\nsec", "ond\nthi"]);
    assert_eq!(
        plain(&output),
        (vec!["first".into(), "second".into(), "thi".into()], 3)
    );
    // Trailing blank lines are not shown, as in the finished output.
    let output = pushed(&["done\n\n\n"]);
    assert_eq!(plain(&output), (vec!["done".into()], 1));
    assert_eq!(plain(&LiveOutput::default()), (Vec::new(), 0));
}

#[test]
fn a_carriage_return_redraws_the_unfinished_line() {
    let mut output = pushed(&["fetch 10%\r"]);
    assert_eq!(lines(&output), ["fetch 10%"]);
    output.push("fetch 50%\r", false);
    assert_eq!(lines(&output), ["fetch 50%"]);
    output.push("fetch 100%\nnext\n", false);
    assert_eq!(lines(&output), ["fetch 100%", "next"]);
    // A CR LF split between two updates ends one line.
    assert_eq!(lines(&pushed(&["a\r", "\nb\n"])), ["a", "b"]);
}

#[test]
fn a_pty_row_written_again_replaces_the_unfinished_one() {
    // The monitor sends a PTY's unfinished row again after `\r ESC[2K`.
    let mut output = pushed(&["$ make\r\nbuilding 1/3"]);
    assert_eq!(lines(&output), ["$ make", "building 1/3"]);
    output.push("\r\x1b[2Kbuilding 2/3", false);
    assert_eq!(lines(&output), ["$ make", "building 2/3"]);
    output.push("\r\x1b[2Kbuilt\r\n", false);
    assert_eq!(lines(&output), ["$ make", "built"]);
}

#[test]
fn an_escape_sequence_split_between_updates_never_shows() {
    let mut output = pushed(&["plain \x1b[3"]);
    assert_eq!(lines(&output), ["plain"]);
    output.push("1mred\x1b[0m\n", false);
    assert_eq!(lines(&output), ["plain red"]);
}

#[test]
fn tabs_are_expanded_as_the_layout_lays_them_out() {
    assert_eq!(lines(&pushed(&["a\tb\n\tc"])), ["a   b", "    c"]);
}

#[test]
fn skipped_output_ends_the_line_before_it_and_shows_as_a_gap() {
    let mut output = pushed(&["kept\nhalf"]);
    output.push("after\n", true);
    assert_eq!(lines(&output), ["kept", "half", "…", "after"]);
}

#[test]
fn the_first_and_the_newest_lines_are_kept_with_how_many_fell_between() {
    let mut output = LiveOutput::default();
    for n in 1..=1000 {
        output.push(&format!("line {n}\n"), false);
    }
    let (shown, count) = plain(&output);
    assert_eq!(count, 1000);
    assert_eq!(shown.len(), HEAD_LINES + 1 + TAIL_LINES);
    assert_eq!(shown[0], "line 1");
    assert_eq!(shown[HEAD_LINES - 1], "line 200");
    assert_eq!(shown[HEAD_LINES], "… +600 lines");
    assert_eq!(shown[HEAD_LINES + 1], "line 801");
    assert_eq!(shown.last().unwrap(), "line 1000");
}

#[test]
fn a_line_is_cut_and_one_that_never_ends_stays_bounded() {
    let output = pushed(&[&format!("{}\n", "é".repeat(MAX_LINE_BYTES))]);
    let line = &lines(&output)[0];
    assert!(
        line.len() <= MAX_LINE_BYTES + '…'.len_utf8(),
        "{}",
        line.len()
    );
    assert!(line.ends_with('…'));

    // A progress bar redrawn without end, and output without a newline.
    let mut output = LiveOutput::default();
    for n in 0..20_000 {
        output.push(&format!("\r\x1b[2K{}%", n % 100), false);
    }
    assert!(
        output.pending.len() <= MAX_PENDING_BYTES,
        "{}",
        output.pending.len()
    );
    assert_eq!(lines(&output), ["99%"]);
    let mut output = LiveOutput::default();
    for _ in 0..20_000 {
        output.push("....", false);
    }
    assert!(
        output.pending.len() <= MAX_PENDING_BYTES,
        "{}",
        output.pending.len()
    );
    assert!(lines(&output)[0].len() <= MAX_LINE_BYTES + '…'.len_utf8());
}

fn red() -> Style {
    Style::default().fg(Color::Red)
}

/// The styled spans of each line: text and style.
fn runs(output: &LiveOutput) -> Vec<Vec<(String, Style)>> {
    output
        .lines()
        .0
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| (span.content.to_string(), span.style))
                .collect()
        })
        .collect()
}

#[test]
fn colours_show_and_a_style_goes_on_across_lines_and_updates() {
    let mut output = pushed(&["plain \x1b[31mred\nstill"]);
    output.push(" red\x1b[0m plain\n", false);
    assert_eq!(
        runs(&output),
        [
            vec![
                ("plain ".to_string(), Style::default()),
                ("red".to_string(), red())
            ],
            vec![
                ("still red".to_string(), red()),
                (" plain".to_string(), Style::default())
            ],
        ]
    );
}

#[test]
fn the_unfinished_line_shows_in_the_style_in_force_and_a_redraw_restyles_it() {
    let mut output = pushed(&["\x1b[32mstart\n", "50%"]);
    let green = Style::default().fg(Color::Green);
    assert_eq!(runs(&output)[1], vec![("50%".to_string(), green)]);
    output.push("\r\x1b[2K\x1b[1;31mfailed\x1b[0m\n", false);
    assert_eq!(
        runs(&output)[1],
        vec![("failed".to_string(), red().add_modifier(Modifier::BOLD))]
    );
}

#[test]
fn a_style_starts_plain_again_after_a_gap() {
    let mut output = pushed(&["\x1b[31mred before\n"]);
    output.push("after\n", true);
    assert_eq!(
        runs(&output)[2],
        vec![("after".to_string(), Style::default())]
    );
}

#[test]
fn a_pty_row_keeps_its_colours_when_written_again() {
    // The monitor writes each styled run of a row between its sequence and
    // a reset, and an unfinished row again after `\r ESC[2K`.
    let mut output = pushed(&["\x1b[0;33mwarn\x1b[0m 1/2"]);
    output.push("\r\x1b[2K\x1b[0;33mwarn\x1b[0m 2/2\n", false);
    let yellow = Style::default().fg(Color::Yellow);
    assert_eq!(
        runs(&output),
        [vec![
            ("warn".to_string(), yellow),
            (" 2/2".to_string(), Style::default())
        ]]
    );
}

#[test]
fn a_collapsed_progress_line_keeps_its_colours_and_the_style_in_force() {
    let mut output = LiveOutput::default();
    output.push("\x1b[36m", false);
    for n in 0..20_000 {
        output.push(&format!("\r{}%", n % 100), false);
    }
    assert!(output.pending.len() <= MAX_PENDING_BYTES);
    let cyan = Style::default().fg(Color::Cyan);
    assert_eq!(runs(&output), [vec![("99%".to_string(), cyan)]]);
    output.push("\rdone\n", false);
    assert_eq!(runs(&output), [vec![("done".to_string(), cyan)]]);
}

/// Codex's `truncated_ansi_sequence_does_not_hide_the_retained_tail`, for
/// this buffer: a title too long for the pending bound, cut, never shows
/// and never hides what follows it.
#[test]
fn a_cut_title_does_not_hide_the_text_after_it() {
    let output = pushed(&[&format!(
        "{}\x1b]0;{}\x07visible-tail",
        "x".repeat(10),
        "hidden".repeat(4_000)
    )]);
    let shown = lines(&output);
    assert_eq!(shown, [format!("{}visible-tail", "x".repeat(10))]);
}
