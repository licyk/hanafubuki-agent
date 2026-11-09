// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

#[test]
fn control_characters_and_whitespace_runs_are_dropped() {
    let sanitized = sanitize_terminal_title(
        "  Project\t|\nWorking\x1b\x07\u{009D}\u{009C} |  Session  ",
        TitleEncoding::Unicode,
    );
    assert_eq!(sanitized, "Project | Working | Session");
}

#[test]
fn bidi_and_invisible_formatting_characters_are_dropped() {
    let sanitized = sanitize_terminal_title(
        "Pro\u{202E}j\u{2066}e\u{200F}c\u{061C}t\u{200B} \u{FEFF}T\u{2060}itle",
        TitleEncoding::Unicode,
    );
    assert_eq!(sanitized, "Project Title");
}

#[test]
fn a_long_title_is_bounded_keeping_a_last_visible_character() {
    let long = "a".repeat(MAX_TERMINAL_TITLE_CHARS + 10);
    assert_eq!(
        sanitize_terminal_title(&long, TitleEncoding::Unicode).len(),
        MAX_TERMINAL_TITLE_CHARS
    );
    let input = format!("{} b", "a".repeat(MAX_TERMINAL_TITLE_CHARS - 1));
    let sanitized = sanitize_terminal_title(&input, TitleEncoding::Unicode);
    assert_eq!(sanitized.len(), MAX_TERMINAL_TITLE_CHARS);
    assert_eq!(sanitized.chars().last(), Some('b'));
}

#[test]
fn under_screen_the_title_is_ascii_and_the_spinner_keeps_turning() {
    let title = " ⠇ | ě[31m session \u{202e}\x07| 日本語 ćafé  ";
    let screen = sanitize_terminal_title(title, TitleEncoding::Screen);
    assert!(screen.is_ascii());
    assert_eq!(screen, "| | ?[31m session | ??? ?af?");
    assert_eq!(
        sanitize_terminal_title(title, TitleEncoding::Unicode),
        "⠇ | ě[31m session | 日本語 ćafé"
    );
    assert_eq!(
        sanitize_terminal_title("\x07\x1b\u{202E}\u{FEFF}", TitleEncoding::Screen),
        ""
    );
}

#[test]
fn the_title_is_pushed_written_once_per_change_and_popped() {
    let mut title = TerminalTitle::new(TitleEncoding::Unicode);
    let mut out = Vec::new();
    title.sync(&mut out, "one\x07").unwrap();
    title.sync(&mut out, "one").unwrap();
    title.sync(&mut out, "two").unwrap();
    // Nothing to show: the terminal's own title back.
    title.sync(&mut out, " \u{200B} ").unwrap();
    title.restore(&mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "\x1b[22;0t\x1b]0;one\x07\x1b]0;two\x07\x1b]0;\x07\x1b[23;0t"
    );

    let mut out = Vec::new();
    title.restore(&mut out).unwrap();
    assert!(out.is_empty(), "nothing was written, nothing is popped");
}
