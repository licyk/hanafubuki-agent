use hanafubuki_agent_process::terminal_size::{TerminalResize, TerminalSize};

use super::{ScreenText, TerminalScreen};

fn screen(cols: u16, rows: u16) -> TerminalScreen {
    TerminalScreen::new(TerminalSize::new(cols, rows))
}

fn text(screen: &mut TerminalScreen, bytes: &[u8]) -> String {
    screen.push(bytes);
    screen.take().text
}

#[test]
fn lines_arrive_in_order_with_their_line_breaks() {
    let mut screen = screen(80, 24);
    assert_eq!(text(&mut screen, b"one\r\ntwo\r\n"), "one\ntwo\n");
    assert_eq!(text(&mut screen, b"three\r\n"), "three\n");
    assert_eq!(text(&mut screen, b""), "");
}

#[test]
fn every_line_scrolled_off_the_screen_arrives() {
    let mut screen = screen(80, 5);
    let output: String = (1..=2000).map(|line| format!("line {line}\r\n")).collect();
    let expected: String = (1..=2000).map(|line| format!("line {line}\n")).collect();
    assert_eq!(text(&mut screen, output.as_bytes()), expected);

    let more: String = (2001..=2010)
        .map(|line| format!("line {line}\r\n"))
        .collect();
    let expected: String = (2001..=2010).map(|line| format!("line {line}\n")).collect();
    assert_eq!(text(&mut screen, more.as_bytes()), expected);
}

#[test]
fn a_line_still_being_written_is_rewritten_by_the_next_take() {
    let mut screen = screen(80, 24);
    screen.push(b"ready\r\nPassword: ");
    assert_eq!(
        screen.take(),
        ScreenText {
            rewrites_last_line: false,
            text: "ready\nPassword:".into()
        }
    );
    screen.push(b"****\r\nwelcome\r\n");
    assert_eq!(
        screen.take(),
        ScreenText {
            rewrites_last_line: true,
            text: "Password: ****\nwelcome\n".into()
        }
    );
}

#[test]
fn a_progress_bar_redrawn_in_place_shows_its_latest_state() {
    let mut screen = screen(80, 24);
    assert_eq!(text(&mut screen, b"copying  10%"), "copying  10%");
    assert_eq!(
        text(&mut screen, b"\rcopying  55%\rcopying 100%\r\ndone\r\n"),
        "copying 100%\ndone\n"
    );
}

#[test]
fn cursor_moves_and_erases_land_where_a_terminal_puts_them() {
    let mut screen = screen(20, 5);
    // ConPTY-style: clear, draw two rows, then repaint the first.
    assert_eq!(
        text(
            &mut screen,
            "\x1b[2J\x1b[Hfirst\r\nsecond\x1b[1;1H\x1b[K中文 ok\x1b[3;1H".as_bytes()
        ),
        "中文 ok\nsecond\n"
    );
}

#[test]
fn a_wrapped_line_stays_one_line() {
    let mut screen = screen(10, 3);
    let line = "x".repeat(25);
    assert_eq!(
        text(&mut screen, format!("{line}\r\nnext\r\n").as_bytes()),
        format!("{line}\nnext\n")
    );
}

#[test]
fn a_sequence_cut_between_pushes_is_held_for_the_rest() {
    let mut screen = screen(80, 24);
    assert_eq!(text(&mut screen, b"red: \x1b[3"), "red:");
    assert_eq!(text(&mut screen, b"1mtext\x1b[0m\r\n"), "red: text\n");
}

#[test]
fn rows_scrolled_off_while_a_sequence_is_open_are_kept() {
    // The scrollback is emptied only between sequences: a title spanning a
    // slice boundary must neither be printed nor cost a row.
    let mut screen = screen(80, 3);
    let mut output = String::new();
    for line in 1..=600 {
        output.push_str(&format!("\x1b]0;title {line}\x07line {line}\r\n"));
    }
    let expected: String = (1..=600).map(|line| format!("line {line}\n")).collect();
    assert_eq!(text(&mut screen, output.as_bytes()), expected);
}

#[test]
fn output_is_replayed_at_the_sizes_it_was_produced_at() {
    let mut screen = screen(80, 24);
    let line = "y".repeat(15);
    screen.push_sized(
        format!("{line}\r\n{line}\r\n").as_bytes(),
        100,
        Some(TerminalSize::new(10, 24)),
        &[TerminalResize {
            offset: 117,
            size: TerminalSize::new(80, 24),
        }],
    );
    // The first line was written at 10 columns and wrapped there, the second
    // at 80. A terminal does not reflow rows when it is resized, so the first
    // line's rows are two lines from then on.
    assert_eq!(
        screen.take().text,
        format!("{}\n{}\n{line}\n", &line[..10], &line[10..])
    );
    assert_eq!(screen.parser.screen().size(), (24, 80));
}
