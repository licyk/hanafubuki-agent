//! Output lines in their styles: colours mapped as the terminal's palette
//! has them, the text always the plain output's, and styles that do not
//! fit their text ignored.

use hanafubuki_agent::{render_styled_text, AgentStyleSpan, AgentTextStyle};
use ratatui::style::{Color, Modifier, Style};

use super::*;

fn rendered(bytes: &[u8]) -> (String, AgentStyledText) {
    let rendered = render_styled_text(bytes, AgentTextStyle::default());
    (rendered.text, rendered.styles)
}

fn spans(lines: &[Line<'static>]) -> Vec<Vec<(String, Style)>> {
    lines
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
fn each_line_shows_its_runs_in_their_styles() {
    let (text, styles) = rendered(b"\x1b[1;31merror\x1b[0m: bad\n\x1b[4;38;5;208mwarm\x1b[0m\n");
    assert_eq!(
        spans(&output_lines(&text, Some(&styles))),
        [
            vec![
                (
                    "error".to_string(),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                ),
                (": bad".to_string(), Style::default()),
            ],
            vec![(
                "warm".to_string(),
                Style::default()
                    .fg(Color::Indexed(208))
                    .add_modifier(Modifier::UNDERLINED)
            )],
        ]
    );
}

#[test]
fn the_sixteen_colours_are_the_terminals_own_and_others_pass_through() {
    for (index, color) in [
        (0, Color::Black),
        (1, Color::Red),
        (7, Color::Gray),
        (8, Color::DarkGray),
        (9, Color::LightRed),
        (15, Color::White),
        (16, Color::Indexed(16)),
        (255, Color::Indexed(255)),
    ] {
        assert_eq!(color_of(AgentTextColor::Indexed(index)), color, "{index}");
    }
    assert_eq!(color_of(AgentTextColor::Rgb(1, 2, 3)), Color::Rgb(1, 2, 3));
    let style = AgentTextStyle {
        fg: None,
        bg: Some(AgentTextColor::Indexed(4)),
        bold: false,
        dim: true,
        italic: true,
        underline: false,
        reverse: true,
    };
    assert_eq!(
        ratatui_style(&style),
        Style::default()
            .bg(Color::Blue)
            .add_modifier(Modifier::DIM | Modifier::ITALIC | Modifier::REVERSED)
    );
}

#[test]
fn without_styles_the_lines_are_the_plain_ones() {
    let text = "a\tb\n\nlast  \n\n";
    let plain = text_lines(text)
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    assert_eq!(output_lines(text, None), plain);
    let (styled_text, styles) = rendered(b"a\tb\n\n\x1b[31mlast\x1b[0m  \n\n");
    assert_eq!(styled_text, text);
    let shown = output_lines(&styled_text, Some(&styles));
    assert_eq!(
        shown.iter().map(line_text).collect::<Vec<_>>(),
        plain.iter().map(line_text).collect::<Vec<_>>()
    );
    // Lines without a run are one plain span, as they are without styles.
    assert_eq!(shown[0], plain[0]);
}

#[test]
fn tabs_are_laid_out_across_runs() {
    let (text, styles) = rendered(b"ab\x1b[32m\tc\x1b[0m\td");
    let shown = output_lines(&text, Some(&styles));
    assert_eq!(line_text(&shown[0]), "ab  c   d");
    assert_eq!(
        spans(&shown)[0],
        [
            ("ab".to_string(), Style::default()),
            ("  c".to_string(), Style::default().fg(Color::Green)),
            ("   d".to_string(), Style::default()),
        ]
    );
}

#[test]
fn styles_of_another_text_show_plain() {
    let (_, styles) = rendered(b"\x1b[31mred\x1b[0m\n");
    // The kernel bounded the output, or a record was changed.
    for other in ["red and more\n", "re", ""] {
        assert_eq!(
            output_lines(other, Some(&styles)),
            text_lines(other)
                .into_iter()
                .map(Line::from)
                .collect::<Vec<_>>(),
            "{other:?}"
        );
    }
    let broken = AgentStyledText {
        text_bytes: 4,
        spans: vec![AgentStyleSpan {
            start: 0,
            end: 9,
            style: AgentTextStyle::default(),
        }],
    };
    assert_eq!(
        output_lines("red\n", Some(&broken)),
        vec![Line::from("red")]
    );
}

#[test]
fn a_line_is_truncated_on_a_character_boundary_across_spans() {
    let mut line = Line::from(vec![Span::raw("ab"), Span::raw("模型"), Span::raw("c")]);
    truncate_line(&mut line, 4);
    assert_eq!(line_text(&line), "ab");
    let mut line = Line::from(vec![Span::raw("ab"), Span::raw("模型"), Span::raw("c")]);
    truncate_line(&mut line, 5);
    assert_eq!(line_text(&line), "ab模");
    assert_eq!(line.spans.len(), 2);
}

/// Hostile output through the renderer and these lines: the text is the
/// plain lines' and nothing a terminal would read as a control remains.
#[test]
fn hostile_output_shows_only_its_text() {
    let pieces: [&[u8]; 14] = [
        b"\x1b[31m",
        b"\x1b[0m",
        b"\x1b]52;c;aGk=\x07",
        b"\x1b[2J",
        b"\x1b[?1049h",
        b"\x1b]0;title\x07",
        b"\r",
        b"\n",
        b"\t",
        b"\x08",
        "模".as_bytes(),
        b"text",
        b"\x1b[38;2;1;2;3;48;5;9;1;7m",
        b"\x1b[",
    ];
    let mut seed: u64 = 0xc0ffee;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..2_000 {
        let mut bytes = Vec::new();
        for _ in 0..next() % 24 {
            bytes.extend_from_slice(pieces[(next() % pieces.len() as u64) as usize]);
        }
        let (text, styles) = rendered(&bytes);
        let shown = output_lines(&text, Some(&styles));
        assert_eq!(
            shown.iter().map(line_text).collect::<Vec<_>>(),
            text_lines(&text),
            "{bytes:?}"
        );
        for line in &shown {
            assert!(!line_text(line).chars().any(char::is_control), "{bytes:?}");
        }
    }
}
