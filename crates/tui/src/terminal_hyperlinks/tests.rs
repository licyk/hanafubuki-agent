// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Codex's annotation, sanitising and emission tests, and hostile
//! destinations and labels: no byte a model writes reaches the terminal as
//! a control sequence.

use std::num::NonZeroU16;

use ratatui::buffer::{Buffer, CellDiffOption, CellWidth};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::emission::osc8_parts;
use super::*;
use crate::wrapping::word_wrap_line_with_source;

fn web(columns: Range<usize>, destination: &str) -> TerminalHyperlink {
    TerminalHyperlink {
        columns,
        destination: destination.to_string(),
    }
}

/// The text under each link of `line`, by destination, in order.
fn linked_text(line: &HyperlinkLine) -> Vec<(String, String)> {
    let text: String = line.line.spans.iter().map(|s| s.content.as_ref()).collect();
    line.hyperlinks
        .iter()
        .map(|link| {
            let mut column = 0;
            let mut under = String::new();
            for grapheme in text.graphemes(true) {
                if link.columns.contains(&column) {
                    under.push_str(grapheme);
                }
                column += display_width(grapheme);
            }
            (link.destination.clone(), under)
        })
        .collect()
}

/// `decorate_spans`' text, with each OSC 8 sequence as `<dest>`/`</>`.
fn decorated(line: &HyperlinkLine) -> String {
    decorate_spans(line)
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
        .replace(OSC8_CLOSE, "</>")
        .replace("\x1b]8;;", "<")
        .replace('\x07', ">")
}

#[test]
fn only_web_destinations_are_linked() {
    assert_eq!(
        web_destination("https://example.com/a").as_deref(),
        Some("https://example.com/a")
    );
    assert_eq!(
        web_destination("http://localhost:8080/x").as_deref(),
        Some("http://localhost:8080/x")
    );
    for destination in [
        "mailto:a@example.com",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "ftp://example.com/file",
        "ssh://host",
        "data:text/html,hi",
        "vscode://file/x",
        "https://",
        "/relative/path",
        "example.com/path",
    ] {
        assert_eq!(web_destination(destination), None, "{destination}");
    }
}

#[test]
fn hostile_destinations_lose_their_control_characters() {
    for (hostile, safe) in [
        ("https://example.com/\u{7}safe", "https://example.com/safe"),
        (
            "https://example.com/\x1b]8;;https://evil.test\x07x",
            "https://example.com/]8;;https://evil.testx",
        ),
        (
            "https://example.com/\x1b[2Jclear",
            "https://example.com/[2Jclear",
        ),
        ("https://example.com/\u{9c}st", "https://example.com/st"),
        ("https://example.com/\u{9b}31m", "https://example.com/31m"),
        ("https://example.com/a\nb\r\tc", "https://example.com/abc"),
        ("https://example.com/\u{7f}del", "https://example.com/del"),
    ] {
        let destination = web_destination(hostile).expect(hostile);
        assert_eq!(destination, safe);
        assert!(!destination.chars().any(char::is_control));
        let mut line = HyperlinkLine::default();
        line.push_span("label".into(), Some(hostile));
        let bytes = decorate_spans(&line)
            .iter()
            .map(|span| span.content.to_string())
            .collect::<String>();
        assert_eq!(bytes, format!("\x1b]8;;{safe}\x07label{OSC8_CLOSE}"));
    }
}

#[test]
fn oversized_destinations_remain_plain_text() {
    let prefix = "https://example.com/";
    let destination = format!(
        "{prefix}{}",
        "a".repeat(MAX_HYPERLINK_DESTINATION_BYTES - prefix.len())
    );
    assert_eq!(web_destination(&destination), Some(destination.clone()));
    let oversized = format!("{destination}é");
    assert_eq!(web_destination(&oversized), None);
    let mut line = HyperlinkLine::default();
    line.push_span("visible".into(), Some(&oversized));
    assert!(line.hyperlinks.is_empty());
    assert_eq!(decorated(&line), "visible");
}

#[test]
fn hostile_labels_reach_the_terminal_without_their_controls() {
    // A label is model text too: only the link's own sequences are escapes.
    let mut line = HyperlinkLine::new(Line::from("a\x1b[31mred\u{9c} "));
    line.push_span(
        "go\x07\x1b]8;;https://evil.test\x07".into(),
        Some("https://example.com"),
    );
    let printed = decorate_spans(&line)
        .iter()
        .map(|span| span.content.to_string())
        .collect::<String>();
    assert_eq!(
        printed,
        format!("a[31mred \x1b]8;;https://example.com\x07go]8;;https://evil.test{OSC8_CLOSE}")
    );
    let escapes = printed.matches('\x1b').count();
    assert_eq!(escapes, 2, "{printed:?}");
}

#[test]
fn discovers_punctuated_web_url_columns() {
    for text in [
        "See (https://example.com/a).",
        "See (https://example.com/a)?",
    ] {
        assert_eq!(
            web_links_in_text(text),
            vec![web(5..26, "https://example.com/a")]
        );
    }
}

#[test]
fn hyperlink_columns_follow_a_long_prefix_without_wrapping() {
    let prefix = "a".repeat(65_536);
    let destination = "https://example.com/long-prefix";
    let text = format!("{prefix} {destination}");
    assert_eq!(
        HyperlinkLine::new(Line::from(text.clone())).width(),
        text.len()
    );
    assert_eq!(
        web_links_in_text(&text),
        vec![web(65_537..65_537 + destination.len(), destination)]
    );
}

#[test]
fn preserves_balanced_parentheses_in_bare_web_urls() {
    for destination in [
        "https://en.wikipedia.org/wiki/Function_(mathematics)",
        "https://en.wikipedia.org/wiki/Function_(mathematics)?q=(alpha)?",
    ] {
        assert_eq!(
            web_links_in_text(&format!("See ({destination}).")),
            vec![web(
                5..5 + usize::from(destination.cell_width()),
                destination
            )]
        );
    }
}

#[test]
fn discovers_many_web_urls_with_incremental_columns() {
    let destination = "https://x.io";
    let token = format!("👩‍💻 ({destination}).\u{2003}");
    let text = token.repeat(50_000);
    let expected = (0..50_000)
        .map(|index| {
            let start = index * display_width(&token) + 4;
            web(start..start + destination.len(), destination)
        })
        .collect::<Vec<_>>();
    assert_eq!(web_links_in_text(&text), expected);
}

#[test]
fn trims_long_runs_of_unmatched_closing_delimiters() {
    let destination = "https://example.com/a_(b)[c]{d}<e>";
    let text = format!("{destination}{}", ")]}>".repeat(50_000));
    assert_eq!(
        web_links_in_text(&text),
        vec![web(0..destination.len(), destination)]
    );
}

#[test]
fn decorates_a_contiguous_web_link_with_one_osc8_pair() {
    let destination = "https://example.com/a/very/long/path";
    let line = HyperlinkLine {
        line: Line::from(destination),
        hyperlinks: vec![web(0..destination.len(), destination)],
    };
    assert_eq!(decorated(&line), format!("<{destination}>{destination}</>"));
    assert_eq!(
        decorate_spans(&HyperlinkLine::new(Line::from("not linked"))),
        vec![Span::from("not linked")]
    );
}

#[test]
fn decorated_hyperlinks_preserve_joined_emoji_columns() {
    let destination = "https://example.test/👩‍💻/path";
    let line = annotate_web_urls_in_line(Line::from(format!("👩‍💻 {destination} tail")));
    assert_eq!(
        decorated(&line),
        format!("👩‍💻 <{destination}>{destination}</> tail")
    );
}

#[test]
fn two_links_side_by_side_close_the_first_before_the_second() {
    let mut line = HyperlinkLine::default();
    line.push_span("one".into(), Some("https://example.com/1"));
    line.push_span("two".into(), Some("https://example.com/2"));
    line.push_span(" three".into(), Some("https://example.com/2"));
    assert_eq!(
        decorated(&line),
        "<https://example.com/1>one</><https://example.com/2>two three</>"
    );
    assert_eq!(line.hyperlinks.len(), 2, "a continued link is one");
}

#[test]
fn wrapping_maps_multiple_links_across_indented_unicode_lines() {
    let text = "alpha 😀here middle there end";
    let first = display_width(&text[..text.find("here").unwrap()]);
    let second = display_width(&text[..text.find("there").unwrap()]);
    let mut source = HyperlinkLine::new(Line::from(text));
    source
        .hyperlinks
        .push(web(first..first + 4, "https://example.com/first"));
    source
        .hyperlinks
        .push(web(second..second + 5, "https://example.com/second"));

    let wrapped = remap_source_wrapped_line(
        &source,
        word_wrap_line_with_source(
            &source.line,
            RtOptions::new(16)
                .initial_indent("  ".into())
                .subsequent_indent("    ".into()),
        ),
    );
    let rows: Vec<_> = wrapped.iter().map(|row| row.line.to_string()).collect();
    assert_eq!(rows, ["  alpha 😀here", "    middle there", "    end"]);
    assert_eq!(
        wrapped[0].hyperlinks,
        vec![web(10..14, "https://example.com/first")]
    );
    assert_eq!(
        wrapped[1].hyperlinks,
        vec![web(11..16, "https://example.com/second")]
    );
    assert!(wrapped[2].hyperlinks.is_empty());
}

#[test]
fn a_link_a_wrap_splits_is_linked_whole_on_each_row() {
    let destination = "https://example.com/docs";
    let mut source = HyperlinkLine::new(Line::from("See "));
    source.push_span("the full documentation page".into(), Some(destination));
    source.push_span(" now".into(), None);
    for width in 4..40 {
        let wrapped =
            remap_source_wrapped_line(&source, word_wrap_line_with_source(&source.line, width));
        let joined: String = wrapped
            .iter()
            .flat_map(linked_text)
            .map(|(link, under)| {
                assert_eq!(link, destination);
                under
            })
            .collect();
        assert_eq!(
            joined.replace(' ', ""),
            "thefulldocumentationpage",
            "{width}"
        );
        for row in &wrapped {
            for link in &row.hyperlinks {
                assert!(link.columns.end <= row.width(), "{width}: {row:?}");
            }
        }
    }
}

#[test]
fn prefixes_and_appends_move_the_links() {
    let mut line = HyperlinkLine::new(Line::from("界 "));
    line.push_span("ｶﾞlink".into(), Some("https://example.com"));
    let mut row = HyperlinkLine::new(Line::from("> "));
    row.append(line.clone());
    assert_eq!(
        linked_text(&row),
        [("https://example.com".into(), "ｶﾞlink".into())]
    );
    let prefixed = prefix_hyperlink_lines(vec![line], "• ".into(), "  ".into());
    assert_eq!(
        linked_text(&prefixed[0]),
        [("https://example.com".into(), "ｶﾞlink".into())]
    );
}

#[test]
fn marked_cells_keep_their_width_for_the_diff() {
    let destination = "https://example.com/dakuten";
    let area = Rect::new(0, 0, 7, 1);
    let previous = Buffer::with_lines(["       "]);
    let mut next = Buffer::empty(area);
    next.set_string(0, 0, "ｶﾞ tail", Style::default().underlined());
    mark_hyperlink_cells(&mut next, Rect::new(0, 0, 2, 1), destination);

    assert_eq!(next[(0, 0)].cell_width(), 2);
    assert!(matches!(
        next[(0, 0)].diff_option,
        CellDiffOption::ForcedWidth(width) if width == NonZeroU16::new(2).unwrap()
    ));
    assert_eq!(osc8_parts(next[(0, 0)].symbol()), Some((destination, "ｶﾞ")));
    assert_eq!(
        previous
            .diff_iter(&next)
            .map(|(x, _, cell)| (
                x,
                osc8_parts(cell.symbol())
                    .map_or(cell.symbol(), |p| p.1)
                    .to_string()
            ))
            .collect::<Vec<_>>(),
        vec![
            (0, "ｶﾞ".to_string()),
            (2, " ".to_string()),
            (3, "t".to_string()),
            (4, "a".to_string()),
            (5, "i".to_string()),
            (6, "l".to_string()),
        ]
    );
}

#[test]
fn marking_stops_at_the_buffer_and_skips_wide_continuations() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
    buf.set_string(0, 0, "漢字", Style::default());
    mark_hyperlink_cells(&mut buf, Rect::new(0, 0, 9, 1), "https://example.com");
    let linked: Vec<_> = (0..4)
        .filter_map(|x| {
            osc8_parts(buf[(x, 0)].symbol()).map(|(_, visible)| (x, visible.to_string()))
        })
        .collect();
    assert_eq!(linked, [(0, "漢".to_string()), (2, "字".to_string())]);
}
