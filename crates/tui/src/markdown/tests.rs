// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;
use ratatui::text::Line;

fn lines_to_strings(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn citations_render_as_plain_text() {
    let src = "Before 【F:/x.rs†L1】\nAfter 【F:/x.rs†L3】\n";
    let mut out = Vec::new();
    append_markdown(src, None, None, &mut out);
    let rendered = lines_to_strings(&out);
    assert_eq!(
        rendered,
        vec![
            "Before 【F:/x.rs†L1】".to_string(),
            "After 【F:/x.rs†L3】".to_string()
        ]
    );
}

#[test]
fn indented_code_blocks_preserve_leading_whitespace() {
    // Basic sanity: indented code with surrounding blank lines should produce the indented line.
    let src = "Before\n\n    code 1\n\nAfter\n";
    let mut out = Vec::new();
    append_markdown(src, None, None, &mut out);
    let lines = lines_to_strings(&out);
    assert_eq!(lines, vec!["Before", "", "    code 1", "", "After"]);
}

#[test]
fn append_markdown_preserves_full_text_line() {
    let src = "Hi! How can I help with codex-rs today? Want me to explore the repo, run tests, or work on a specific change?\n";
    let mut out = Vec::new();
    append_markdown(src, None, None, &mut out);
    assert_eq!(
        out.len(),
        1,
        "expected a single rendered line for plain text"
    );
    let rendered: String = out
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(
        rendered,
        "Hi! How can I help with codex-rs today? Want me to explore the repo, run tests, or work on a specific change?"
    );
}

#[test]
fn append_markdown_matches_tui_markdown_for_ordered_item() {
    let mut out = Vec::new();
    append_markdown("1. Tight item\n", None, None, &mut out);
    let lines = lines_to_strings(&out);
    assert_eq!(lines, vec!["1. Tight item".to_string()]);
}

#[test]
fn append_markdown_keeps_ordered_list_line_unsplit_in_context() {
    let src = "Loose vs. tight list items:\n1. Tight item\n";
    let mut out = Vec::new();
    append_markdown(src, None, None, &mut out);

    let lines = lines_to_strings(&out);

    // Expect to find the ordered list line rendered as a single line,
    // not split into a marker-only line followed by the text.
    assert!(
        lines.iter().any(|s| s == "1. Tight item"),
        "expected '1. Tight item' rendered as a single line; got: {lines:?}"
    );
    assert!(
        !lines
            .windows(2)
            .any(|w| w[0].trim_end() == "1." && w[1] == "Tight item"),
        "did not expect a split into ['1.', 'Tight item']; got: {lines:?}"
    );
}

fn agent_lines(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    append_markdown_agent(src, None, None, &mut out);
    lines_to_strings(&out)
}

#[test]
fn append_markdown_agent_unwraps_markdown_fences_for_table_rendering() {
    let rendered = agent_lines("```markdown\n| A | B |\n|---|---|\n| 1 | 2 |\n```\n");
    assert!(
        rendered.iter().any(|line| line.contains('━')),
        "{rendered:?}"
    );
    assert!(
        rendered.iter().any(|line| line.contains(" 1      2")),
        "{rendered:?}"
    );
}

#[test]
fn append_markdown_agent_unwraps_markdown_fences_for_no_outer_table_rendering() {
    let rendered = agent_lines(
        "```md\nCol A | Col B | Col C\n--- | --- | ---\nx | y | z\n10 | 20 | 30\n```\n",
    );
    assert!(rendered.iter().any(|line| line.contains('━')));
    assert!(rendered
        .iter()
        .any(|line| line.contains(" Col A    Col B    Col C")));
    assert!(!rendered
        .iter()
        .any(|line| line.trim() == "Col A | Col B | Col C"));
}

#[test]
fn append_markdown_agent_unwraps_markdown_fences_for_two_column_no_outer_table() {
    let rendered = agent_lines("```md\nA | B\n--- | ---\nleft | right\n```\n");
    assert!(rendered.iter().any(|line| line.contains('━')));
    assert!(rendered.iter().any(|line| line.contains(" left    right")));
    assert!(!rendered.iter().any(|line| line.trim() == "A | B"));
}

#[test]
fn append_markdown_agent_unwraps_markdown_fences_for_single_column_table() {
    let rendered = agent_lines("```md\n| Only |\n|---|\n| value |\n```\n");
    assert!(rendered.iter().any(|line| line.contains('━')));
    assert!(!rendered.iter().any(|line| line.trim() == "| Only |"));
}

#[test]
fn append_markdown_agent_keeps_non_markdown_fences_as_code() {
    let rendered = agent_lines("```rust\n| A | B |\n|---|---|\n| 1 | 2 |\n```\n");
    assert_eq!(rendered, vec!["| A | B |", "|---|---|", "| 1 | 2 |"]);
}

#[test]
fn append_markdown_agent_unwraps_blockquoted_markdown_fence_table() {
    let src = "> ```markdown\n> | A | B |\n> |---|---|\n> | 1 | 2 |\n> ```\n";
    let rendered = unwrap_markdown_fences(src);
    assert!(!rendered.contains("```"), "{rendered:?}");
}

#[test]
fn append_markdown_agent_keeps_non_blockquoted_markdown_fence_with_blockquote_table_example() {
    let src = "```markdown\n> | A | B |\n> |---|---|\n> | 1 | 2 |\n```\n";
    assert_eq!(unwrap_markdown_fences(src), src);
}

#[test]
fn append_markdown_agent_keeps_markdown_fence_when_content_is_not_table() {
    assert_eq!(
        agent_lines("```markdown\n**bold**\n```\n"),
        vec!["**bold**"]
    );
}

#[test]
fn unwrap_markdown_fences_repro_keeps_fence_without_header_delimiter_pair() {
    let src = "```markdown\n| A | B |\nnot a delimiter row\n| --- | --- |\n# Heading\n```\n";
    assert_eq!(unwrap_markdown_fences(src), src);
}

#[test]
fn append_markdown_agent_keeps_markdown_fence_with_blank_line_between_header_and_delimiter() {
    let src = "```markdown\n| A | B |\n\n|---|---|\n| 1 | 2 |\n```\n";
    assert_eq!(unwrap_markdown_fences(src), src);
}

#[test]
fn an_unclosed_markdown_fence_stays_code() {
    let src = "Intro\n\n```md\n| A | B |\n|---|---|\n";
    assert_eq!(unwrap_markdown_fences(src), src);
}

/// Plans and reasoning keep their fences: only an answer is unwrapped.
#[test]
fn append_markdown_keeps_markdown_fences() {
    let mut out = Vec::new();
    append_markdown("```md\n| A |\n|---|\n| 1 |\n```\n", None, None, &mut out);
    assert_eq!(lines_to_strings(&out), vec!["| A |", "|---|", "| 1 |"]);
}
