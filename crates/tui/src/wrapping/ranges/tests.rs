// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use textwrap::{Options, WordSeparator};

use super::*;

#[test]
fn narrow_wrap_ranges_preserve_compound_graphemes() {
    assert_eq!(wrap_ranges_trim("👩‍💻e\u{301}x", 2), vec![0..11, 11..15],);
    for text in ["\u{301}\u{302}", " \u{301}\u{302}"] {
        assert_eq!(wrap_ranges_trim(text, 2), vec![0..text.len()]);
    }
    assert_eq!(wrap_ranges("\u{301}\u{302}", 2), vec![0..5],);
}

#[test]
fn map_owned_wrapped_line_to_range_recovers_on_non_prefix_mismatch() {
    // Match source chars first, then introduce a non-penalty mismatch.
    // The function should recover and return the mapped prefix range.
    let range = map_owned_wrapped_line_to_range("hello world", 0, "helloX", "");
    assert_eq!(range, 0..5);
}

#[test]
fn borrowed_slice_range_rejects_slices_outside_source_text() {
    let text = "test message";
    let external = String::from("test");

    assert_eq!(borrowed_slice_range(text, &external), None);

    let fallback = map_owned_wrapped_line_to_range(text, 0, &external, "");
    assert_eq!(fallback, 0..4);
}

#[test]
fn map_owned_wrapped_line_to_range_indent_coincides_with_source() {
    // When the synthetic indent prefix starts with a character that also
    // appears at the current source position, the mapper must not confuse
    // the indent char for a source match.  Here the indent is "- " and the
    // source text also starts with "-", so a naive char-by-char match would
    // consume the source "-" for the indent "-", set saw_source_char too
    // early, then break on the space — returning 0..1 instead of the full
    // first word.
    let text = "- item one and some more words";
    // Simulate what textwrap would produce for the first continuation line
    // when subsequent_indent = "- ": it prepends "- " to the source slice.
    let range = map_owned_wrapped_line_to_range(text, 0, "- - item one", "- ");
    // The mapper should skip the synthetic "- " prefix and map "- item one"
    // back to source bytes 0..10.
    assert_eq!(range, 0..10);
}

#[test]
fn wrap_ranges_indent_prefix_coincides_with_source_char() {
    // End-to-end: source text starts with the same character as the indent
    // prefix.  wrap_ranges must still reconstruct the full source.
    let text = "- first item is long enough to wrap around";
    let opts = || {
        textwrap::Options::new(16)
            .initial_indent("- ")
            .subsequent_indent("- ")
    };
    let ranges = wrap_ranges(text, opts());
    assert!(!ranges.is_empty());

    let mut rebuilt = String::new();
    let mut cursor = 0usize;
    for range in ranges {
        let start = range.start.max(cursor).min(text.len());
        let end = range.end.min(text.len());
        if start < end {
            rebuilt.push_str(&text[start..end]);
        }
        cursor = cursor.max(end);
    }
    assert_eq!(rebuilt, text);
}

#[test]
fn wrap_ranges_count_halfwidth_sound_marks_without_changing_byte_offsets() {
    for (text, width, expected) in [
        ("abｶﾞc", 4, &["abｶﾞ", "c"][..]),
        ("abｶﾞc", 3, &["ab", "ｶﾞc"][..]),
        ("ﾞab", 2, &["ﾞa", "b"][..]),
        ("a ﾞb", 2, &["a", "ﾞb"][..]),
        ("a ﾟb", 2, &["a", "ﾟb"][..]),
        ("ｶﾞﾞx", 3, &["ｶﾞﾞ", "x"][..]),
        ("界ﾞx", 3, &["界ﾞ", "x"][..]),
        ("ｶﾞﾞab", 2, &["ｶﾞﾞ", "ab"][..]),
        ("界ﾞab", 2, &["界ﾞ", "ab"][..]),
        ("abｶﾞﾞcd", 2, &["ab", "ｶﾞﾞ", "cd"][..]),
        ("ab界ﾞcd", 2, &["ab", "界ﾞ", "cd"][..]),
    ] {
        let ranges = wrap_ranges_trim(text, Options::new(width));
        let wrapped = ranges
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<Vec<_>>();
        assert_eq!(wrapped, expected);
    }

    for (text, emoji, sound_mark) in [("a👨‍👩 ﾞ", "👨‍👩", "ﾞ"), ("a👍🏻 ﾟ", "👍🏻", "ﾟ")]
    {
        for options in [
            Options::new(2),
            Options::new(2).word_separator(WordSeparator::AsciiSpace),
        ] {
            let ranges = wrap_ranges_trim(text, options);
            let wrapped = ranges
                .iter()
                .map(|range| &text[range.clone()])
                .collect::<Vec<_>>();

            assert_eq!(wrapped, ["a", emoji, sound_mark]);
        }
    }

    for grapheme in ["ｶﾞﾞ", "界ﾞ"] {
        for width in [1, 2] {
            let ranges = wrap_ranges(grapheme, Options::new(width));
            assert_eq!(
                ranges,
                std::iter::once(0..grapheme.len() + 1).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn wrap_ranges_preserve_crlf_source_boundaries_without_splitting_graphemes() {
    for prefix in ["ｶﾞ", "ﾊﾟ"] {
        let text = format!("{prefix}\r\nnext");

        for word_separator in [
            WordSeparator::UnicodeBreakProperties,
            WordSeparator::AsciiSpace,
        ] {
            for line_ending in [textwrap::LineEnding::LF, textwrap::LineEnding::CRLF] {
                let options = Options::new(4)
                    .line_ending(line_ending)
                    .word_separator(word_separator)
                    .wrap_algorithm(textwrap::WrapAlgorithm::FirstFit);
                let first_end = prefix.len() + usize::from(line_ending == textwrap::LineEnding::LF);
                let second_start = prefix.len() + "\r\n".len();

                let ranges = wrap_ranges(&text, options.clone());
                assert_eq!(ranges, [0..first_end + 1, second_start..text.len() + 1]);
                for range in &ranges {
                    assert!(text.get(range.start..range.end - 1).is_some());
                }

                let trimmed = wrap_ranges_trim(&text, options);
                assert_eq!(trimmed, [0..first_end, second_start..text.len()]);
            }
        }
    }
}

#[test]
fn map_owned_wrapped_line_to_range_repro_overconsumes_repeated_prefix_patterns() {
    let text = "- - foo";
    let opts = textwrap::Options::new(3)
        .initial_indent("- ")
        .subsequent_indent("- ")
        .word_separator(textwrap::WordSeparator::AsciiSpace)
        .break_words(false);
    let wrapped = textwrap::wrap(text, opts);
    let Some(line) = wrapped.first() else {
        panic!("expected at least one wrapped line");
    };

    let mapped = map_owned_wrapped_line_to_range(text, 0, line.as_ref(), "- ");
    let expected_len = line
        .as_ref()
        .strip_prefix("- ")
        .unwrap_or(line.as_ref())
        .len();
    let mapped_len = mapped.end.saturating_sub(mapped.start);
    assert!(
        mapped_len <= expected_len,
        "overconsumed source: text={text:?} line={line:?} mapped={mapped:?} expected_len={expected_len}"
    );
}

#[test]
fn wrap_ranges_recovers_with_non_space_indents() {
    let text = "The quick brown fox jumps over the lazy dog";
    let wrapped = textwrap::wrap(
        text,
        textwrap::Options::new(12)
            .initial_indent("* ")
            .subsequent_indent("  "),
    );
    assert!(
        wrapped
            .iter()
            .any(|line| matches!(line, std::borrow::Cow::Owned(_))),
        "expected textwrap to produce owned lines with synthetic indent prefixes"
    );

    let ranges = wrap_ranges(
        text,
        textwrap::Options::new(12)
            .initial_indent("* ")
            .subsequent_indent("  "),
    );
    assert!(!ranges.is_empty());

    // wrap_ranges returns cursor-oriented ranges that may overlap by one byte;
    // rebuild with cursor progression to validate full source coverage.
    let mut rebuilt = String::new();
    let mut cursor = 0usize;
    for range in ranges {
        let start = range.start.max(cursor).min(text.len());
        let end = range.end.min(text.len());
        if start < end {
            rebuilt.push_str(&text[start..end]);
        }
        cursor = cursor.max(end);
    }

    assert_eq!(rebuilt, text);
}

#[test]
fn wrap_ranges_trim_handles_owned_lines_with_penalty_char() {
    fn split_every_char(word: &str) -> Vec<usize> {
        word.char_indices().skip(1).map(|(idx, _)| idx).collect()
    }

    let text = "a_very_long_token_without_spaces";
    let opts = Options::new(8)
        .word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::Custom(split_every_char))
        .break_words(false);

    let ranges = wrap_ranges_trim(text, opts);
    let rebuilt = ranges
        .iter()
        .map(|range| &text[range.clone()])
        .collect::<String>();

    assert_eq!(rebuilt, text);
    assert!(ranges.len() > 1, "expected wrapped ranges, got: {ranges:?}");
}
