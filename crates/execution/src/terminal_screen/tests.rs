use hanafubuki_agent_process::terminal_size::{TerminalResize, TerminalSize};

use hanafubuki_agent_foundation::terminal_text::{TextColor, TextStyle};

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
        plain(screen.take()),
        (false, "ready\nPassword:".to_string())
    );
    screen.push(b"****\r\nwelcome\r\n");
    assert_eq!(
        plain(screen.take()),
        (true, "Password: ****\nwelcome\n".to_string())
    );
}

fn plain(taken: ScreenText) -> (bool, String) {
    assert!(taken.styles.fits(&taken.text));
    (taken.rewrites_last_line, taken.text)
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

#[test]
fn wide_wrapped_and_full_width_lines_survive_the_scrollback_being_emptied() {
    // Enough rows to empty the scrollback several times, with characters cut
    // between slices, rows that wrap, and rows exactly as wide as the screen.
    // A wide character that finds one column left moves to the next row
    // without marking the row wrapped (vt100 does as tmux does), so the wide
    // rows here fill each row exactly.
    let mut screen = screen(40, 5);
    let lines = (1..=600)
        .map(|line| match line % 4 {
            0 => format!("{line} 中文的行"),
            1 => format!("{line:03} {}", "长".repeat(50)),
            2 => format!("{line:0>40}"),
            _ => format!("{line} {}", "x".repeat(130)),
        })
        .collect::<Vec<_>>();
    let output: String = lines.iter().map(|line| format!("{line}\r\n")).collect();
    let expected: String = lines.iter().map(|line| format!("{line}\n")).collect();
    assert_eq!(text(&mut screen, output.as_bytes()), expected);
}

#[test]
fn the_alternate_screen_leaves_the_main_one_as_it_was() {
    let mut screen = screen(40, 5);
    let mut output = String::from("before\r\n\x1b[?1049h");
    for line in 1..=600 {
        output.push_str(&format!("full-screen {line}\r\n"));
    }
    output.push_str("\x1b[?1049lafter\r\n");
    assert_eq!(text(&mut screen, output.as_bytes()), "before\nafter\n");
}

fn runs(taken: &ScreenText) -> Vec<(&str, TextStyle)> {
    assert!(taken.styles.fits(&taken.text), "{taken:?}");
    taken
        .styles
        .spans
        .iter()
        .map(|span| (&taken.text[span.start..span.end], span.style))
        .collect()
}

fn fg(index: u8) -> TextStyle {
    TextStyle {
        fg: Some(TextColor::Indexed(index)),
        ..TextStyle::default()
    }
}

#[test]
fn a_rows_cells_keep_their_colours_and_attributes() {
    let mut screen = screen(80, 24);
    screen.push(
        b"\x1b[1;31merror\x1b[0m: \x1b[38;2;1;2;3;4mrgb\x1b[0m \x1b[7;3;2;48;5;200mx\x1b[0m\r\n",
    );
    let taken = screen.take();
    assert_eq!(taken.text, "error: rgb x\n");
    assert_eq!(
        runs(&taken),
        vec![
            (
                "error",
                TextStyle {
                    bold: true,
                    ..fg(1)
                }
            ),
            (
                "rgb",
                TextStyle {
                    fg: Some(TextColor::Rgb(1, 2, 3)),
                    underline: true,
                    ..TextStyle::default()
                }
            ),
            (
                "x",
                TextStyle {
                    bg: Some(TextColor::Indexed(200)),
                    reverse: true,
                    italic: true,
                    dim: true,
                    ..TextStyle::default()
                }
            ),
        ]
    );
}

#[test]
fn a_row_whose_colour_alone_changed_is_not_taken_again() {
    // The text the model reads would otherwise repeat a row it already
    // has, only because its colour changed.
    let mut screen = screen(80, 24);
    assert_eq!(text(&mut screen, b"building\r\n"), "building\n");
    screen.push(b"\x1b[1A\r\x1b[32mbuilding\x1b[0m\r\n");
    let taken = screen.take();
    assert_eq!(taken.text, "");
    assert!(taken.styles.spans.is_empty());
}

#[test]
fn rows_scrolled_off_keep_their_styles() {
    let mut screen = screen(40, 5);
    let output: String = (1..=600)
        .map(|line| format!("\x1b[3{}mline {line}\x1b[0m\r\n", line % 8))
        .collect();
    screen.push(output.as_bytes());
    let taken = screen.take();
    let runs = runs(&taken);
    assert_eq!(runs.len(), 600);
    for ((text, style), line) in runs.iter().zip(1..=600u32) {
        assert_eq!(*text, format!("line {line}"));
        assert_eq!(*style, fg((line % 8) as u8));
    }
}

#[test]
fn wide_characters_gaps_and_blanks_are_styled_where_they_stand() {
    let mut screen = screen(20, 5);
    screen.push("\x1b[32m中\x1b[0m\x1b[3Cx \x1b[41m  \x1b[0my\x1b[44m   \x1b[0m\r\n".as_bytes());
    let taken = screen.take();
    assert_eq!(taken.text, "中   x   y\n");
    assert_eq!(
        runs(&taken),
        vec![
            ("中", fg(2)),
            (
                "  ",
                TextStyle {
                    bg: Some(TextColor::Indexed(1)),
                    ..TextStyle::default()
                }
            ),
        ]
    );
}

#[test]
fn hostile_output_leaves_rows_whose_styles_fit_them() {
    let pieces: [&[u8]; 16] = [
        b"\x1b[",
        b"31m",
        b"1;4;7m",
        b"2J",
        b"H",
        b"5;3H",
        b"K",
        b"\r\n",
        b"\x08",
        "中".as_bytes(),
        b"text ",
        b"\x1b[?1049h",
        b"\x1b[?1049l",
        b"\x1b]0;t\x07",
        b"38;2;1;2;3m",
        b"\x1b[0m",
    ];
    let mut seed: u64 = 0xfeed_beef;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..300 {
        let mut screen = screen(12, 4);
        for _ in 0..8 {
            let mut bytes = Vec::new();
            for _ in 0..next() % 30 {
                bytes.extend_from_slice(pieces[(next() % pieces.len() as u64) as usize]);
            }
            screen.push(&bytes);
            let taken = screen.take();
            runs(&taken);
        }
    }
}
