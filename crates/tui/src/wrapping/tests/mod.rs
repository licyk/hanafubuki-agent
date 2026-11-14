// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use ratatui::style::Color;
use ratatui::style::Stylize;
use std::string::ToString;

use super::urls::url_and_prose_tokens;
use super::*;

fn concat_line(line: &Line) -> String {
    line.spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>()
}

fn adaptive_wrap_line<'a>(line: &'a Line<'a>, options: RtOptions<'a>) -> Vec<Line<'a>> {
    adaptive_wrap_line_with_source(line, options)
        .into_iter()
        .map(|wrapped| wrapped.line)
        .collect()
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
    let wrapped = word_wrap_lines(&lines, 40);
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

#[test]
fn ascii_space_separator_with_no_hyphenation_keeps_url_intact() {
    let line = Line::from(
        "http://example.com/long-url-with-dashes-wider-than-terminal-window/blah-blah-blah-text/more-gibberish-text",
    );
    let opts = RtOptions::new(24)
        .word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::NoHyphenation)
        .break_words(false);

    let out = word_wrap_line(&line, opts);

    assert_eq!(out.len(), 1);
    assert_eq!(
        concat_line(&out[0]),
        "http://example.com/long-url-with-dashes-wider-than-terminal-window/blah-blah-blah-text/more-gibberish-text"
    );
}

#[test]
fn text_contains_url_like_matches_expected_tokens() {
    let positives = [
        "https://example.com/a/b",
        "ftp://host/path",
        "www.example.com/path?x=1",
        "example.test/path#frag",
        "localhost:3000/api",
        "127.0.0.1:8080/health",
        "(https://example.com/wrapped-in-parens)",
    ];

    for text in positives {
        assert!(
            url_and_prose_tokens(text).0,
            "expected URL-like match for {text:?}"
        );
    }
}

#[test]
fn text_contains_url_like_rejects_non_urls() {
    let negatives = [
        "src/main.rs",
        "foo/bar",
        "key:value",
        "just-some-text-with-dashes",
        "hello.world", // no path/query/fragment and no www
    ];

    for text in negatives {
        assert!(
            !url_and_prose_tokens(text).0,
            "did not expect URL-like match for {text:?}"
        );
    }
}

#[test]
fn line_contains_url_like_checks_across_spans() {
    let line = Line::from(vec![
        "see ".into(),
        "https://example.com/a/very/long/path".cyan(),
        " for details".into(),
    ]);

    assert!(url_and_prose_tokens(&concat_line(&line)).0);
}

#[test]
fn line_has_mixed_url_and_non_url_tokens_detects_prose_plus_url() {
    let line = Line::from("see https://example.com/path for details");
    assert_eq!(url_and_prose_tokens(&concat_line(&line)), (true, true));
}

#[test]
fn line_has_mixed_url_and_non_url_tokens_ignores_pipe_prefix() {
    let line = Line::from(vec!["  │ ".into(), "https://example.com/path".into()]);
    assert_eq!(url_and_prose_tokens(&concat_line(&line)), (true, false));
}

#[test]
fn line_has_mixed_url_and_non_url_tokens_ignores_ordered_list_marker() {
    let line = Line::from("1. https://example.com/path");
    assert_eq!(url_and_prose_tokens(&concat_line(&line)), (true, false));
}

#[test]
fn text_contains_url_like_accepts_custom_scheme_with_separator() {
    assert!(url_and_prose_tokens("myapp://open/some/path").0);
}

#[test]
fn text_contains_url_like_rejects_invalid_ports() {
    assert!(!url_and_prose_tokens("localhost:99999/path").0);
    assert!(!url_and_prose_tokens("example.com:abc/path").0);
}

#[test]
fn adaptive_wrap_line_keeps_long_url_like_token_intact() {
    let line = Line::from("example.test/a-very-long-path-with-many-segments-and-query?x=1&y=2");
    let out = adaptive_wrap_line(&line, RtOptions::new(20));
    assert_eq!(out.len(), 1);
    assert_eq!(
        concat_line(&out[0]),
        "example.test/a-very-long-path-with-many-segments-and-query?x=1&y=2"
    );
}

#[test]
fn adaptive_wrap_line_preserves_default_behavior_for_non_url_tokens() {
    let line = Line::from("a_very_long_token_without_spaces_to_force_wrapping");
    let out = adaptive_wrap_line(&line, RtOptions::new(20));
    assert!(
        out.len() > 1,
        "expected non-url token to wrap with default options"
    );
}

#[test]
fn adaptive_wrap_line_mixed_line_keeps_regular_words_intact() {
    let line = Line::from(
        "see https://example.com/path and keep strikethrough intact while wrapping prose",
    );
    let out = adaptive_wrap_line(&line, RtOptions::new(36));
    let joined = out.iter().map(concat_line).collect::<Vec<_>>().join("\n");

    assert_eq!(
        joined,
        "see https://example.com/path and\nkeep strikethrough intact while\nwrapping prose"
    );
}

#[test]
fn adaptive_wrap_line_keeps_url_split_across_styled_spans_intact() {
    let line = Line::from(vec![
        "see ".red(),
        "https://exa".cyan(),
        "mple.com/path".magenta(),
        " now".green(),
    ]);
    let out = adaptive_wrap_line(&line, RtOptions::new(10));

    assert_eq!(
        out,
        vec![
            Line::from("see".red()),
            Line::from(vec!["https://exa".cyan(), "mple.com/path".magenta()]),
            Line::from("now".green()),
        ]
    );
}

#[test]
fn adaptive_wrap_line_mixed_line_wraps_long_non_url_token() {
    let long_non_url = "a_very_long_token_without_spaces_to_force_wrapping";
    let line = Line::from(format!("see https://ex.com {long_non_url}"));
    let out = adaptive_wrap_line(&line, RtOptions::new(24));

    assert!(
        out.iter()
            .any(|line| concat_line(line).contains("https://ex.com")),
        "expected URL token to remain present, got: {out:?}"
    );
    assert!(
        !out.iter()
            .any(|line| concat_line(line).contains(long_non_url)),
        "expected long non-url token to wrap on mixed lines, got: {out:?}"
    );
}

#[test]
fn adaptive_wrap_line_mixed_line_counts_leading_spaces_before_first_word() {
    let line = Line::from("      abcdefgh https://x.co");
    let out = adaptive_wrap_line(&line, RtOptions::new(10).subsequent_indent("      ".into()));
    let rendered = out.iter().map(concat_line).collect::<Vec<_>>();

    assert_eq!(
        rendered[..2],
        ["      abcd".to_string(), "      efgh".to_string()]
    );
}

#[test]
fn adaptive_wrap_line_mixed_line_resplits_long_token_for_continuation_width() {
    let line = Line::from("abcdefghijklmnopqrst https://x.co");
    let out = adaptive_wrap_line(&line, RtOptions::new(10).subsequent_indent("    ".into()));
    let rendered = out.iter().map(concat_line).collect::<Vec<_>>();

    assert_eq!(
        rendered[..3],
        [
            "abcdefghij".to_string(),
            "    klmnop".to_string(),
            "    qrst".to_string(),
        ]
    );
}

#[test]
fn adaptive_wrap_line_mixed_url_counts_halfwidth_sound_marks() {
    let line = Line::from("ｶﾞﾊﾟtail https://x.co");
    let out = adaptive_wrap_line(&line, RtOptions::new(4));
    let rendered = out.iter().map(concat_line).collect::<Vec<_>>();

    assert_eq!(rendered, ["ｶﾞﾊﾟ", "tail", "https://x.co"]);
}

#[test]
fn adaptive_wrap_line_mixed_url_makes_progress_for_an_indivisible_grapheme() {
    let line = Line::from("ｶﾞ https://x.co");
    let out = adaptive_wrap_line(&line, RtOptions::new(1));
    let rendered = out.iter().map(concat_line).collect::<Vec<_>>();

    assert_eq!(rendered, ["ｶﾞ", "https://x.co"]);
}

#[test]
fn a_marker_stays_with_a_url_wider_than_its_row() {
    let url = "https://example.com/a/rather/long/path/that/wraps/at/forty/columns";
    for prefix in ["• ", "1. ", "> ", "☐ ", "  ", "    • "] {
        let text = format!("{prefix}{url}");
        let line = Line::from(text.clone());
        let out = adaptive_wrap_line(&line, RtOptions::new(40));
        assert_eq!(
            out.iter().map(concat_line).collect::<Vec<_>>(),
            [text],
            "{prefix:?}"
        );
    }
    // A URL that fits on a row of its own keeps it whole, the marker
    // before it.
    let line = Line::from("• https://example.com/fits/in/forty/col/x");
    let out = adaptive_wrap_line(&line, RtOptions::new(40));
    assert_eq!(
        out.iter().map(concat_line).collect::<Vec<_>>(),
        ["•", "https://example.com/fits/in/forty/col/x"]
    );
    // Blank before it, it keeps the indent on its row.
    let line = Line::from("  https://example.com/fits/in/forty/col/x");
    let rows = adaptive_wrap_line_with_source(&line, RtOptions::new(40));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].range, 0..line.width());
}

mod reuse;
