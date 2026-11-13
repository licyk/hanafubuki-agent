// Derived from OpenAI Codex (https://github.com/openai/codex), the tests
// of `parse_thread_title` in `codex-rs/tui/src/app/thread_title_tests.rs`
// at commit `57a808a392`. Copyright 2025 OpenAI. Licensed under the Apache
// License, Version 2.0 (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the cases that apply to a decoded title;
// a title over the bound is cut, and control characters are dropped.

use super::*;

#[test]
fn generation_prompt_truncation_preserves_utf8_boundaries() {
    let prompt = format!("{}tail", "问".repeat(TITLE_PROMPT_MAX_BYTES));
    let bounded = bounded_prompt(&prompt);
    assert!(bounded.len() <= TITLE_PROMPT_MAX_BYTES);
    assert!(bounded.is_char_boundary(bounded.len()));
}

#[test]
fn normalizes_generated_title_whitespace() {
    assert_eq!(
        normalize_generated_title("  Fix  \n\t login   errors  ").unwrap(),
        "Fix login errors"
    );
}

#[test]
fn removes_wrapping_quotes_and_trailing_punctuation_from_generated_titles() {
    for title in [
        r#""Fix login errors!""#,
        "'Fix login errors?'",
        "`Fix login errors.`",
        "“Fix login errors!”",
    ] {
        assert_eq!(
            normalize_generated_title(title).unwrap(),
            "Fix login errors",
            "{title}"
        );
    }
}

#[test]
fn preserves_meaningful_leading_punctuation_in_generated_titles() {
    for (title, expected) in [
        (".NET migration.", ".NET migration"),
        ("!important styling!", "!important styling"),
    ] {
        assert_eq!(normalize_generated_title(title).unwrap(), expected);
    }
}

#[test]
fn rejects_empty_generated_titles() {
    for title in ["", "  \t  ", "\"\"", "...", "\u{7}"] {
        assert!(normalize_generated_title(title).is_err(), "{title:?}");
    }
}

#[test]
fn truncates_generated_titles_without_splitting_unicode() {
    let expected = "🚀".repeat(AUTOMATIC_AGENT_TITLE_MAX_CHARS);
    assert_eq!(
        normalize_generated_title(&format!("{expected}x")).unwrap(),
        expected
    );
    // Cut, no longer refused.
    assert_eq!(
        normalize_generated_title(&"x".repeat(AUTOMATIC_AGENT_TITLE_MAX_CHARS + 1)).unwrap(),
        "x".repeat(AUTOMATIC_AGENT_TITLE_MAX_CHARS)
    );
}
