//! The styled renderer: which SGR attributes and colours it keeps, that
//! everything else is dropped as the plain renderer drops it, that its text
//! is the plain renderer's whatever the input, and that its styles survive
//! the round trip through terminal text.

use super::super::{
    render_plain_text, render_styled_text, sgr_sequence, styled_terminal_text, StyleSpan,
    TextColor, TextStyle, TextStyles, MAX_STYLE_SPANS,
};

fn styled(bytes: &[u8]) -> (String, Vec<(String, TextStyle)>) {
    let rendered = render_styled_text(bytes, TextStyle::default());
    let runs = rendered
        .styles
        .spans
        .iter()
        .map(|span| (rendered.text[span.start..span.end].to_string(), span.style))
        .collect();
    (rendered.text, runs)
}

fn fg(color: TextColor) -> TextStyle {
    TextStyle {
        fg: Some(color),
        ..TextStyle::default()
    }
}

fn red() -> TextStyle {
    fg(TextColor::Indexed(1))
}

#[test]
fn a_coloured_word_is_one_run_and_reset_ends_it() {
    let (text, runs) = styled(b"plain \x1b[31mred\x1b[0m plain");
    assert_eq!(text, "plain red plain");
    assert_eq!(runs, vec![("red".to_string(), red())]);
}

#[test]
fn every_kept_attribute_turns_on_and_off() {
    for (on, off, style) in [
        (
            "1",
            "22",
            TextStyle {
                bold: true,
                ..TextStyle::default()
            },
        ),
        (
            "2",
            "22",
            TextStyle {
                dim: true,
                ..TextStyle::default()
            },
        ),
        (
            "3",
            "23",
            TextStyle {
                italic: true,
                ..TextStyle::default()
            },
        ),
        (
            "4",
            "24",
            TextStyle {
                underline: true,
                ..TextStyle::default()
            },
        ),
        (
            "7",
            "27",
            TextStyle {
                reverse: true,
                ..TextStyle::default()
            },
        ),
    ] {
        let input = format!("a\x1b[{on}mb\x1b[{off}mc");
        assert_eq!(
            styled(input.as_bytes()),
            ("abc".to_string(), vec![("b".to_string(), style)]),
            "{on}/{off}"
        );
    }
}

#[test]
fn twenty_two_ends_bold_and_dim_together() {
    let (_, runs) = styled(b"\x1b[1;2mx\x1b[22my");
    assert_eq!(
        runs,
        vec![(
            "x".to_string(),
            TextStyle {
                bold: true,
                dim: true,
                ..TextStyle::default()
            }
        )]
    );
}

#[test]
fn the_sixteen_colours_are_palette_indexes_for_both_planes() {
    for code in 30..=37u8 {
        let input = format!("\x1b[{code}mx");
        assert_eq!(
            styled(input.as_bytes()).1[0].1,
            fg(TextColor::Indexed(code - 30))
        );
    }
    for code in 90..=97u8 {
        let input = format!("\x1b[{code}mx");
        assert_eq!(
            styled(input.as_bytes()).1[0].1,
            fg(TextColor::Indexed(code - 90 + 8))
        );
    }
    for (code, index) in [(40u8, 0u8), (47, 7), (100, 8), (107, 15)] {
        let input = format!("\x1b[{code}mx");
        assert_eq!(
            styled(input.as_bytes()).1[0].1,
            TextStyle {
                bg: Some(TextColor::Indexed(index)),
                ..TextStyle::default()
            }
        );
    }
}

#[test]
fn the_256_colour_palette_in_both_separators() {
    for index in [0u8, 15, 16, 208, 255] {
        for form in [format!("38;5;{index}"), format!("38:5:{index}")] {
            let input = format!("\x1b[{form}mx");
            assert_eq!(
                styled(input.as_bytes()).1[0].1,
                fg(TextColor::Indexed(index)),
                "{form}"
            );
        }
    }
    assert_eq!(
        styled(b"\x1b[48;5;17mx").1[0].1,
        TextStyle {
            bg: Some(TextColor::Indexed(17)),
            ..TextStyle::default()
        }
    );
}

#[test]
fn truecolour_in_every_form_and_both_planes_in_one_sequence() {
    for form in ["38;2;1;2;3", "38:2:1:2:3", "38:2::1:2:3", "38:2:0:1:2:3"] {
        let input = format!("\x1b[{form}mx");
        assert_eq!(
            styled(input.as_bytes()).1[0].1,
            fg(TextColor::Rgb(1, 2, 3)),
            "{form}"
        );
    }
    // 34 characters of parameters: past the 32 cursor movement reads.
    let (_, runs) = styled(b"\x1b[38;2;255;255;255;48;2;255;254;253;1mx");
    assert_eq!(
        runs[0].1,
        TextStyle {
            fg: Some(TextColor::Rgb(255, 255, 255)),
            bg: Some(TextColor::Rgb(255, 254, 253)),
            bold: true,
            ..TextStyle::default()
        }
    );
}

#[test]
fn defaults_and_resets() {
    let (_, runs) = styled(b"\x1b[31;44mx\x1b[39my\x1b[49mz");
    assert_eq!(
        runs,
        vec![
            (
                "x".to_string(),
                TextStyle {
                    fg: Some(TextColor::Indexed(1)),
                    bg: Some(TextColor::Indexed(4)),
                    ..TextStyle::default()
                }
            ),
            (
                "y".to_string(),
                TextStyle {
                    bg: Some(TextColor::Indexed(4)),
                    ..TextStyle::default()
                }
            ),
        ]
    );
    // An empty sequence and an empty parameter are resets.
    assert_eq!(styled(b"\x1b[31mx\x1b[my").1.len(), 1);
    assert_eq!(styled(b"\x1b[1mx\x1b[;31my").1[1].1, red());
    assert_eq!(styled(b"\x1b[4mx\x1b[4:0my").1.len(), 1);
}

#[test]
fn unknown_codes_are_skipped_and_the_rest_applies() {
    assert_eq!(styled(b"\x1b[200;31mx").1[0].1, red());
    assert_eq!(styled(b"\x1b[9999999;31mx").1[0].1, red());
    // An underline colour's parts are not read as codes of their own.
    assert_eq!(styled(b"\x1b[58;5;1;31mx").1[0].1, red());
    assert_eq!(styled(b"\x1b[58;2;1;2;3mx").1, vec![]);
    // A colour with a part missing or out of range sets nothing.
    assert_eq!(styled(b"\x1b[38;5mx").1, vec![]);
    assert_eq!(styled(b"\x1b[38;5;256mx").1, vec![]);
    assert_eq!(styled(b"\x1b[38;2;1;2mx").1, vec![]);
}

#[test]
fn dropped_attributes_show_plain() {
    for code in ["5", "6", "8", "9", "53", "21", "60"] {
        let input = format!("\x1b[{code}mvisible");
        assert_eq!(
            styled(input.as_bytes()),
            ("visible".to_string(), vec![]),
            "{code}"
        );
    }
}

#[test]
fn a_style_lives_on_across_lines_and_each_run_ends_with_its_line() {
    let rendered = render_styled_text(b"\x1b[32mone\ntwo\x1b[0m\nthree", TextStyle::default());
    assert_eq!(rendered.text, "one\ntwo\nthree");
    let green = fg(TextColor::Indexed(2));
    assert_eq!(
        rendered.styles.spans,
        vec![
            StyleSpan {
                start: 0,
                end: 3,
                style: green
            },
            StyleSpan {
                start: 4,
                end: 7,
                style: green
            },
        ]
    );
    assert_eq!(rendered.pen, TextStyle::default());
}

#[test]
fn the_style_at_the_end_carries_into_the_next_read() {
    let first = render_styled_text(b"a\x1b[31mb", TextStyle::default());
    assert_eq!(first.pen, red());
    let second = render_styled_text(b"c\x1b[0md", first.pen);
    assert_eq!(second.text, "cd");
    assert_eq!(
        second.styles.spans,
        vec![StyleSpan {
            start: 0,
            end: 1,
            style: red()
        }]
    );
}

#[test]
fn a_sequence_split_by_a_read_styles_nothing_until_it_is_whole() {
    // A reader holds an unfinished sequence back (`complete_prefix_len`);
    // given alone, the cut one is dropped and styles nothing.
    assert_eq!(styled(b"x\x1b[3"), ("x".to_string(), vec![]));
    assert_eq!(styled(b"x\x1b[31").1, vec![]);
}

#[test]
fn a_carriage_return_overwrite_takes_the_new_cells_style() {
    let (text, runs) = styled(b"\x1b[31mwaiting\r\x1b[32mdone\x1b[0m");
    assert_eq!(text, "doneing");
    assert_eq!(
        runs,
        vec![
            ("done".to_string(), fg(TextColor::Indexed(2))),
            ("ing".to_string(), red()),
        ]
    );
    let (text, runs) = styled(b"\x1b[31mprogress 50%\r\x1b[2K\x1b[0mdone");
    assert_eq!((text.as_str(), runs), ("done", vec![]));
}

#[test]
fn wide_characters_and_marks_take_the_style_of_their_cell() {
    let (text, runs) = styled("\x1b[33m模型e\u{301}\x1b[0m!".as_bytes());
    assert_eq!(text, "模型e\u{301}!");
    assert_eq!(
        runs,
        vec![("模型e\u{301}".to_string(), fg(TextColor::Indexed(3)))]
    );
}

#[test]
fn non_sgr_sequences_and_private_m_style_nothing() {
    for input in [
        &b"\x1b[>4;2mx"[..],
        b"\x1b[?1mx",
        b"\x1b[1 mx",
        b"\x1b]52;c;MzFt\x07x",
        b"\x1b[31qx",
    ] {
        assert_eq!(styled(input), ("x".to_string(), vec![]), "{input:?}");
    }
}

#[test]
fn an_overlong_sgr_sequence_changes_nothing() {
    let input = format!("\x1b[31mx\x1b[{}32my", "0;".repeat(80));
    assert_eq!(styled(input.as_bytes()).1, vec![("xy".to_string(), red())]);
}

#[test]
fn a_style_change_at_every_character_keeps_a_bounded_number_of_runs() {
    let input = (0..MAX_STYLE_SPANS + 500)
        .map(|index| format!("\x1b[3{}mx", 1 + index % 2))
        .collect::<String>();
    let rendered = render_styled_text(input.as_bytes(), TextStyle::default());
    assert_eq!(rendered.text.len(), MAX_STYLE_SPANS + 500);
    assert_eq!(rendered.styles.spans.len(), MAX_STYLE_SPANS);
    assert!(rendered.styles.fits(&rendered.text));
}

#[test]
fn styles_fit_only_their_own_text() {
    let rendered = render_styled_text("\x1b[31m模型\x1b[0m".as_bytes(), TextStyle::default());
    assert!(rendered.styles.fits("模型"));
    assert!(!rendered.styles.fits("模型 and more"));
    let mut cut = rendered.styles.clone();
    cut.spans[0].end = 1;
    assert!(!cut.fits("模型"), "not at a character boundary");
    let mut overlapping = TextStyles {
        text_bytes: 4,
        spans: vec![
            StyleSpan {
                start: 0,
                end: 3,
                style: red(),
            },
            StyleSpan {
                start: 2,
                end: 4,
                style: red(),
            },
        ],
    };
    assert!(!overlapping.fits("abcd"));
    overlapping.spans[1].start = 3;
    assert!(overlapping.fits("abcd"));
}

#[test]
fn styled_terminal_text_reads_back_as_the_same_text_and_styles() {
    let input =
        "\x1b[1;31merror\x1b[0m: \x1b[38;5;208mwarm\x1b[48;2;1;2;3;3mrgb\x1b[m\n\x1b[7;4;90mlast";
    let first = render_styled_text(input.as_bytes(), TextStyle::default());
    let encoded = styled_terminal_text(&first.text, &first.styles);
    let second = render_styled_text(encoded.as_bytes(), TextStyle::default());
    assert_eq!(second.text, first.text);
    assert_eq!(second.styles, first.styles);
    assert_eq!(second.pen, TextStyle::default(), "every run is closed");
    assert_eq!(render_plain_text(encoded.as_bytes()), first.text);
}

#[test]
fn every_style_reads_back_from_its_sequence() {
    let colors = [
        None,
        Some(TextColor::Indexed(0)),
        Some(TextColor::Indexed(7)),
        Some(TextColor::Indexed(8)),
        Some(TextColor::Indexed(15)),
        Some(TextColor::Indexed(16)),
        Some(TextColor::Indexed(255)),
        Some(TextColor::Rgb(0, 128, 255)),
    ];
    for fg in colors {
        for bg in colors {
            for flags in 0..32u8 {
                let style = TextStyle {
                    fg,
                    bg,
                    bold: flags & 1 != 0,
                    dim: flags & 2 != 0,
                    italic: flags & 4 != 0,
                    underline: flags & 8 != 0,
                    reverse: flags & 16 != 0,
                };
                let input = format!("\x1b[1;3;34m{}x", sgr_sequence(&style));
                assert_eq!(
                    render_styled_text(input.as_bytes(), TextStyle::default()).pen,
                    style
                );
            }
        }
    }
}

#[test]
fn styles_decode_from_their_json_and_tolerate_unknown_fields() {
    let styles = render_styled_text(b"\x1b[1;38;2;1;2;3;48;5;9mx", TextStyle::default()).styles;
    let json = serde_json::to_value(&styles).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "textBytes": 1,
            "spans": [{
                "start": 0,
                "end": 1,
                "style": { "fg": { "rgb": [1, 2, 3] }, "bg": { "indexed": 9 }, "bold": true }
            }]
        })
    );
    let mut extended = json.clone();
    extended["future"] = serde_json::json!(true);
    extended["spans"][0]["style"]["blink"] = serde_json::json!(true);
    assert_eq!(
        serde_json::from_value::<TextStyles>(extended).unwrap(),
        styles
    );
}

/// Random input from the bytes sequences are made of, and every prefix of
/// it: the text is the plain renderer's, nothing but `\n` and `\t` of the
/// control characters survives, and the styles fit the text.
#[test]
fn hostile_and_cut_input_never_panics_and_leaves_only_text_and_fitting_styles() {
    let pieces: [&[u8]; 34] = [
        b"\x1b",
        b"[",
        b"]",
        b"3",
        b"1",
        b";",
        b":",
        b"m",
        b"K",
        b"C",
        b"D",
        b"G",
        b"H",
        b"J",
        b"\r",
        b"\n",
        b"\x08",
        b"\t",
        b"a",
        "模".as_bytes(),
        b"\xff",
        b"?",
        b">",
        b"2",
        b"8",
        b"5",
        b"\x07",
        b"\\",
        b"P",
        b" ",
        "\u{301}".as_bytes(),
        b"\x9b",
        b"52;c;aGk=",
        b"38;2;255;255;255;48;2;1;2;3",
    ];
    let mut seed: u64 = 0x5eed_1234_abcd;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..3_000 {
        let mut input = Vec::new();
        for _ in 0..next() % 48 {
            input.extend_from_slice(pieces[(next() % pieces.len() as u64) as usize]);
        }
        for cut in 0..=input.len() {
            let bytes = &input[..cut];
            let rendered = render_styled_text(bytes, TextStyle::default());
            assert_eq!(rendered.text, render_plain_text(bytes), "{bytes:?}");
            assert!(
                !rendered
                    .text
                    .chars()
                    .any(|ch| ch.is_control() && ch != '\n' && ch != '\t'),
                "{bytes:?} -> {:?}",
                rendered.text
            );
            assert!(rendered.styles.fits(&rendered.text), "{bytes:?}");
            assert!(rendered
                .styles
                .spans
                .iter()
                .all(|span| !rendered.text[span.start..span.end].contains('\n')
                    && span.style != TextStyle::default()));
        }
    }
}
