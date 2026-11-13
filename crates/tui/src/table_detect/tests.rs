// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

#[test]
fn parse_table_segments_basic() {
    assert_eq!(
        parse_table_segments("| A | B | C |"),
        Some(vec!["A", "B", "C"])
    );
}

#[test]
fn parse_table_segments_no_outer_pipes() {
    assert_eq!(parse_table_segments("A | B | C"), Some(vec!["A", "B", "C"]));
}

#[test]
fn parse_table_segments_no_leading_pipe() {
    assert_eq!(
        parse_table_segments("A | B | C |"),
        Some(vec!["A", "B", "C"])
    );
}

#[test]
fn parse_table_segments_no_trailing_pipe() {
    assert_eq!(
        parse_table_segments("| A | B | C"),
        Some(vec!["A", "B", "C"])
    );
}

#[test]
fn parse_table_segments_single_segment_is_allowed() {
    assert_eq!(parse_table_segments("| only |"), Some(vec!["only"]));
}

#[test]
fn parse_table_segments_without_pipe_returns_none() {
    assert_eq!(parse_table_segments("just text"), None);
}

#[test]
fn parse_table_segments_empty_returns_none() {
    assert_eq!(parse_table_segments(""), None);
    assert_eq!(parse_table_segments("   "), None);
}

#[test]
fn parse_table_segments_escaped_pipe() {
    assert_eq!(
        parse_table_segments(r"| A \| B | C |"),
        Some(vec![r"A \| B", "C"])
    );
    // A trailing backslash escapes nothing and splits nothing.
    assert_eq!(parse_table_segments(r"A | B\"), Some(vec!["A", r"B\"]));
}

#[test]
fn is_table_delimiter_segment_valid() {
    assert!(is_table_delimiter_segment("---"));
    assert!(is_table_delimiter_segment(":---"));
    assert!(is_table_delimiter_segment("---:"));
    assert!(is_table_delimiter_segment(":---:"));
    assert!(is_table_delimiter_segment(":-------:"));
}

#[test]
fn is_table_delimiter_segment_invalid() {
    assert!(!is_table_delimiter_segment(""));
    assert!(!is_table_delimiter_segment("--"));
    assert!(!is_table_delimiter_segment("abc"));
    assert!(!is_table_delimiter_segment(":--"));
}

#[test]
fn is_table_delimiter_line_valid() {
    assert!(is_table_delimiter_line("| --- | --- |"));
    assert!(is_table_delimiter_line("|:---:|---:|"));
    assert!(is_table_delimiter_line("--- | --- | ---"));
}

#[test]
fn is_table_delimiter_line_invalid() {
    assert!(!is_table_delimiter_line("| A | B |"));
    assert!(!is_table_delimiter_line("| -- | -- |"));
}

#[test]
fn is_table_header_line_valid() {
    assert!(is_table_header_line("| A | B |"));
    assert!(is_table_header_line("Name | Value"));
}

#[test]
fn is_table_header_line_all_empty_segments() {
    assert!(!is_table_header_line("| | |"));
}

#[test]
fn fence_tracker_outside_by_default() {
    let tracker = FenceTracker::new();
    assert_eq!(tracker.kind(), FenceKind::Outside);
}

#[test]
fn fence_tracker_opens_and_closes_backtick_fence() {
    let mut tracker = FenceTracker::new();
    tracker.advance("```rust");
    assert_eq!(tracker.kind(), FenceKind::Other);
    tracker.advance("let x = 1;");
    assert_eq!(tracker.kind(), FenceKind::Other);
    tracker.advance("```");
    assert_eq!(tracker.kind(), FenceKind::Outside);
}

#[test]
fn fence_tracker_opens_and_closes_tilde_fence() {
    let mut tracker = FenceTracker::new();
    tracker.advance("~~~python");
    assert_eq!(tracker.kind(), FenceKind::Other);
    tracker.advance("~~~");
    assert_eq!(tracker.kind(), FenceKind::Outside);
}

#[test]
fn fence_tracker_markdown_fence() {
    let mut tracker = FenceTracker::new();
    tracker.advance("```md");
    assert_eq!(tracker.kind(), FenceKind::Markdown);
    tracker.advance("| A | B |");
    assert_eq!(tracker.kind(), FenceKind::Markdown);
    tracker.advance("```");
    assert_eq!(tracker.kind(), FenceKind::Outside);
}

#[test]
fn fence_tracker_markdown_case_insensitive() {
    let mut tracker = FenceTracker::new();
    tracker.advance("```Markdown");
    assert_eq!(tracker.kind(), FenceKind::Markdown);
    tracker.advance("```");
    assert_eq!(tracker.kind(), FenceKind::Outside);
}

#[test]
fn fence_tracker_nested_shorter_marker_does_not_close() {
    let mut tracker = FenceTracker::new();
    tracker.advance("````sh");
    assert_eq!(tracker.kind(), FenceKind::Other);
    tracker.advance("```");
    assert_eq!(tracker.kind(), FenceKind::Other);
    tracker.advance("````");
    assert_eq!(tracker.kind(), FenceKind::Outside);
}

#[test]
fn fence_tracker_mismatched_char_does_not_close() {
    let mut tracker = FenceTracker::new();
    tracker.advance("```sh");
    assert_eq!(tracker.kind(), FenceKind::Other);
    tracker.advance("~~~");
    assert_eq!(tracker.kind(), FenceKind::Other);
    tracker.advance("```");
    assert_eq!(tracker.kind(), FenceKind::Outside);
}

#[test]
fn fence_tracker_indented_4_spaces_ignored() {
    let mut tracker = FenceTracker::new();
    tracker.advance("    ```sh");
    assert_eq!(tracker.kind(), FenceKind::Outside);
}

#[test]
fn fence_tracker_blockquote_prefix_stripped() {
    let mut tracker = FenceTracker::new();
    tracker.advance("> ```sh");
    assert_eq!(tracker.kind(), FenceKind::Other);
    tracker.advance("> ```");
    assert_eq!(tracker.kind(), FenceKind::Outside);
}

#[test]
fn fence_tracker_close_with_trailing_content_does_not_close() {
    let mut tracker = FenceTracker::new();
    tracker.advance("```sh");
    assert_eq!(tracker.kind(), FenceKind::Other);
    tracker.advance("``` extra");
    assert_eq!(tracker.kind(), FenceKind::Other);
    tracker.advance("```");
    assert_eq!(tracker.kind(), FenceKind::Outside);
}

#[test]
fn parse_fence_marker_backtick() {
    assert_eq!(parse_fence_marker("```rust"), Some(('`', 3)));
    assert_eq!(parse_fence_marker("````"), Some(('`', 4)));
}

#[test]
fn parse_fence_marker_tilde() {
    assert_eq!(parse_fence_marker("~~~python"), Some(('~', 3)));
}

#[test]
fn parse_fence_marker_too_short() {
    assert_eq!(parse_fence_marker("``"), None);
    assert_eq!(parse_fence_marker("~~"), None);
}

#[test]
fn parse_fence_marker_not_fence() {
    assert_eq!(parse_fence_marker("hello"), None);
    assert_eq!(parse_fence_marker(""), None);
}

#[test]
fn is_markdown_fence_info_basic() {
    assert!(is_markdown_fence_info("```md", 3));
    assert!(is_markdown_fence_info("```markdown", 3));
    assert!(is_markdown_fence_info("```MD", 3));
    assert!(!is_markdown_fence_info("```rust", 3));
    assert!(!is_markdown_fence_info("```", 3));
    // The info string alone, as pulldown-cmark reports it.
    assert!(is_markdown_fence_info("markdown title=x", 0));
}

#[test]
fn strip_blockquote_prefix_basic() {
    assert_eq!(strip_blockquote_prefix("> hello"), "hello");
    assert_eq!(strip_blockquote_prefix("> > nested"), "nested");
    assert_eq!(strip_blockquote_prefix("no prefix"), "no prefix");
}
