// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;
use ratatui::style::Color;
use ratatui::style::Stylize;
use std::string::ToString;

fn concat_line(line: &Line) -> String {
    line.spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>()
}

#[test]
fn trivial_unstyled_no_indents_wide_width() {
    let line = Line::from("hello");
    let out = word_wrap_line(&line, 10);
    assert_eq!(out.len(), 1);
    assert_eq!(concat_line(&out[0]), "hello");
}

#[test]
fn simple_unstyled_wrap_narrow_width() {
    let line = Line::from("hello world");
    let out = word_wrap_line(&line, 5);
    assert_eq!(out.len(), 2);
    assert_eq!(concat_line(&out[0]), "hello");
    assert_eq!(concat_line(&out[1]), "world");
}

#[test]
fn simple_styled_wrap_preserves_styles() {
    let line = Line::from(vec!["hello ".red(), "world".into()]);
    let out = word_wrap_line(&line, 6);
    assert_eq!(out.len(), 2);
    // First line should carry the red style
    assert_eq!(concat_line(&out[0]), "hello");
    assert_eq!(out[0].spans.len(), 1);
    assert_eq!(out[0].spans[0].style.fg, Some(Color::Red));
    // Second line is unstyled
    assert_eq!(concat_line(&out[1]), "world");
    assert_eq!(out[1].spans.len(), 1);
    assert_eq!(out[1].spans[0].style.fg, None);
}

#[test]
fn with_initial_and_subsequent_indents() {
    let opts = RtOptions::new(8)
        .initial_indent(Line::from("- "))
        .subsequent_indent(Line::from("  "));
    let line = Line::from("hello world foo");
    let out = word_wrap_line(&line, opts);
    // Expect three lines with proper prefixes
    assert!(concat_line(&out[0]).starts_with("- "));
    assert!(concat_line(&out[1]).starts_with("  "));
    assert!(concat_line(&out[2]).starts_with("  "));
    // And content roughly segmented
    assert_eq!(concat_line(&out[0]), "- hello");
    assert_eq!(concat_line(&out[1]), "  world");
    assert_eq!(concat_line(&out[2]), "  foo");
}

#[test]
fn empty_initial_indent_subsequent_spaces() {
    let opts = RtOptions::new(8)
        .initial_indent(Line::from(""))
        .subsequent_indent(Line::from("    "));
    let line = Line::from("hello world foobar");
    let out = word_wrap_line(&line, opts);
    assert!(concat_line(&out[0]).starts_with("hello"));
    for l in &out[1..] {
        assert!(concat_line(l).starts_with("    "));
    }
}

#[test]
fn empty_input_yields_single_empty_line() {
    let line = Line::from("");
    let out = word_wrap_line(&line, 10);
    assert_eq!(out.len(), 1);
    assert_eq!(concat_line(&out[0]), "");
}

#[test]
fn leading_spaces_preserved_on_first_line() {
    let line = Line::from("   hello");
    let out = word_wrap_line(&line, 8);
    assert_eq!(out.len(), 1);
    assert_eq!(concat_line(&out[0]), "   hello");
}

#[test]
fn multiple_spaces_between_words_dont_start_next_line_with_spaces() {
    let line = Line::from("hello   world");
    let out = word_wrap_line(&line, 8);
    assert_eq!(out.len(), 2);
    assert_eq!(concat_line(&out[0]), "hello");
    assert_eq!(concat_line(&out[1]), "world");
}

#[test]
fn break_words_false_allows_overflow_for_long_word() {
    let opts = RtOptions::new(5).break_words(false);
    let line = Line::from("supercalifragilistic");
    let out = word_wrap_line(&line, opts);
    assert_eq!(out.len(), 1);
    assert_eq!(concat_line(&out[0]), "supercalifragilistic");
}

#[test]
fn hyphen_splitter_breaks_at_hyphen() {
    let line = Line::from("hello-world");
    let out = word_wrap_line(&line, 7);
    assert_eq!(out.len(), 2);
    assert_eq!(concat_line(&out[0]), "hello-");
    assert_eq!(concat_line(&out[1]), "world");
}

#[test]
fn indent_consumes_width_leaving_one_char_space() {
    let opts = RtOptions::new(4)
        .initial_indent(Line::from(">>>>"))
        .subsequent_indent(Line::from("--"));
    let line = Line::from("hello");
    let out = word_wrap_line(&line, opts);
    assert_eq!(out.len(), 3);
    assert_eq!(concat_line(&out[0]), ">>>>h");
    assert_eq!(concat_line(&out[1]), "--el");
    assert_eq!(concat_line(&out[2]), "--lo");
}

#[test]
fn wide_unicode_wraps_by_display_width() {
    let line = Line::from("😀😀😀");
    let out = word_wrap_line(&line, 4);
    assert_eq!(out.len(), 2);
    assert_eq!(concat_line(&out[0]), "😀😀");
    assert_eq!(concat_line(&out[1]), "😀");
}

#[test]
fn styled_split_within_span_preserves_style() {
    use ratatui::style::Stylize;
    let line = Line::from(vec!["abcd".red()]);
    let out = word_wrap_line(&line, 2);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].spans.len(), 1);
    assert_eq!(out[1].spans.len(), 1);
    assert_eq!(out[0].spans[0].style.fg, Some(Color::Red));
    assert_eq!(out[1].spans[0].style.fg, Some(Color::Red));
    assert_eq!(concat_line(&out[0]), "ab");
    assert_eq!(concat_line(&out[1]), "cd");
}

#[test]
fn wrap_lines_applies_initial_indent_only_once() {
    let opts = RtOptions::new(8)
        .initial_indent(Line::from("- "))
        .subsequent_indent(Line::from("  "));

    let lines = vec![Line::from("hello world"), Line::from("foo bar baz")];
    let out = word_wrap_lines(lines, opts);

    // Expect: first line prefixed with "- ", subsequent wrapped pieces with "  "
    // and for the second input line, there should be no "- " prefix on its first piece
    let rendered: Vec<String> = out.iter().map(concat_line).collect();
    assert!(rendered[0].starts_with("- "));
    for r in rendered.iter().skip(1) {
        assert!(r.starts_with("  "));
    }
}

#[test]
fn wrap_lines_without_indents_is_concat_of_single_wraps() {
    let lines = vec![Line::from("hello"), Line::from("world!")];
    let out = word_wrap_lines(lines, 10);
    let rendered: Vec<String> = out.iter().map(concat_line).collect();
    assert_eq!(rendered, vec!["hello", "world!"]);
}

#[test]
fn wrap_lines_borrowed_applies_initial_indent_only_once() {
    let opts = RtOptions::new(8)
        .initial_indent(Line::from("- "))
        .subsequent_indent(Line::from("  "));

    let lines = [Line::from("hello world"), Line::from("foo bar baz")];
    let out = word_wrap_lines_borrowed(lines.iter(), opts);

    let rendered: Vec<String> = out.iter().map(concat_line).collect();
    assert!(rendered.first().unwrap().starts_with("- "));
    for r in rendered.iter().skip(1) {
        assert!(r.starts_with("  "));
    }
}

#[test]
fn wrap_lines_borrowed_without_indents_is_concat_of_single_wraps() {
    let lines = [Line::from("hello"), Line::from("world!")];
    let out = word_wrap_lines_borrowed(lines.iter(), 10);
    let rendered: Vec<String> = out.iter().map(concat_line).collect();
    assert_eq!(rendered, vec!["hello", "world!"]);
}

#[test]
fn wrap_lines_accepts_borrowed_iterators() {
    let lines = [Line::from("hello world"), Line::from("foo bar baz")];
    let out = word_wrap_lines(lines, 10);
    let rendered: Vec<String> = out.iter().map(concat_line).collect();
    assert_eq!(rendered, vec!["hello", "world", "foo bar", "baz"]);
}

#[test]
fn wrap_lines_accepts_str_slices() {
    let lines = ["hello world", "goodnight moon"];
    let out = word_wrap_lines(lines, 12);
    let rendered: Vec<String> = out.iter().map(concat_line).collect();
    assert_eq!(rendered, vec!["hello world", "goodnight", "moon"]);
}

#[test]
fn line_height_counts_double_width_emoji() {
    let line = "😀😀😀".into(); // each emoji ~ width 2
    assert_eq!(word_wrap_line(&line, 4).len(), 2);
    assert_eq!(word_wrap_line(&line, 2).len(), 3);
    assert_eq!(word_wrap_line(&line, 6).len(), 1);
}

#[test]
fn word_wrap_does_not_split_words_simple_english() {
    let sample = "Years passed, and Willowmere thrived in peace and friendship. Mira’s herb garden flourished with both ordinary and enchanted plants, and travelers spoke of the kindness of the woman who tended them.";
    let line = Line::from(sample);
    let lines = [line];
    // Force small width to exercise wrapping at spaces.
    let wrapped = word_wrap_lines_borrowed(&lines, 40);
    let joined: String = wrapped
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        joined,
        r#"Years passed, and Willowmere thrived in
peace and friendship. Mira’s herb garden
flourished with both ordinary and
enchanted plants, and travelers spoke of
the kindness of the woman who tended
them."#
    );
}
