// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What shows while a stream arrives: the committed rows, then the tail —
//! a table as far as it arrived, a ```` ```md ```` document, a list marker
//! alone, the line still arriving.

use super::{full_render, lines_to_plain_strings, stream_controller};
use crate::streaming::controller::TestStream;

/// The rows committed and the tail after `chunks`.
fn shown(chunks: &[&str], width: Option<usize>) -> (Vec<String>, Vec<String>) {
    let mut stream = TestStream::new(width, "/repo");
    let mut committed = Vec::new();
    for chunk in chunks {
        stream.push(chunk);
        committed.extend(stream.commit());
    }
    (
        lines_to_plain_strings(&committed),
        lines_to_plain_strings(&stream.tail()),
    )
}

#[test]
fn a_table_shows_its_rows_as_they_arrive_and_commits_once_complete() {
    let (committed, tail) = shown(
        &["Intro\n\n", "| A | B |\n", "|---|---|\n", "| 1 | 2 |\n"],
        Some(40),
    );
    assert_eq!(committed, vec!["Intro"]);
    assert_eq!(tail, vec!["", " A      B", "━━━━━  ━━━━━", " 1      2"]);

    // A wider row lays the table out again, in the tail.
    let (committed, tail) = shown(
        &[
            "Intro\n\n| A | B |\n|---|---|\n| 1 | 2 |\n",
            "| a longer cell | x |\n",
        ],
        Some(40),
    );
    assert_eq!(committed, vec!["Intro"]);
    assert_eq!(tail[1], " A                B");

    // A row's unterminated cell waits for its newline.
    let (_, tail) = shown(&["| A | B |\n|---|---|\n| partial"], Some(40));
    assert!(!tail.join("\n").contains("partial"), "{tail:?}");

    // Once a block follows, the table is committed.
    let (committed, tail) = shown(
        &["Intro\n\n| A | B |\n|---|---|\n| 1 | 2 |\n", "\nAfter\n"],
        Some(40),
    );
    assert_eq!(
        committed,
        vec![
            "Intro",
            "",
            " A      B",
            "━━━━━  ━━━━━",
            " 1      2",
            "",
            "After"
        ]
    );
    assert!(tail.is_empty());
}

#[test]
fn a_long_markdown_fence_shows_as_it_arrives_and_is_committed_once_decided() {
    let mut chunks = vec!["Intro.\n\n", "```md\n", "# A long document\n\n"];
    let lines: Vec<String> = (0..40)
        .map(|line| format!("Line {line} of the document.\n"))
        .collect();
    chunks.extend(lines.iter().map(String::as_str));
    let (committed, tail) = shown(&chunks, Some(60));
    assert_eq!(committed, vec!["Intro."]);
    assert!(tail
        .iter()
        .any(|line| line.contains("Line 39 of the document.")));
    assert!(tail.iter().any(|line| line.contains("# A long document")));

    // It closes holding a table: unwrapped, then committed whole.
    chunks.extend([
        "\n| A | B |\n",
        "|---|---|\n",
        "| 1 | 2 |\n",
        "```\n",
        "\nAfter.\n",
    ]);
    let (committed, tail) = shown(&chunks, Some(60));
    let source: String = chunks.concat();
    assert_eq!(committed, full_render(&source, Some(60)));
    assert!(tail.is_empty());
    assert!(committed.iter().any(|line| line.contains('━')));
}

#[test]
fn a_list_marker_alone_waits_on_its_row_for_its_text() {
    let (committed, tail) = shown(&["1. Tight\n\n", "2.\n"], Some(40));
    assert_eq!(committed, vec!["1. Tight"]);
    assert_eq!(tail, vec!["2. "]);
    let (committed, tail) = shown(&["1. Tight\n\n2.\n", "   Loose\n"], Some(40));
    assert_eq!(committed, vec!["1. Tight", "2. Loose"]);
    assert!(tail.is_empty());
}

#[test]
fn a_code_fence_is_committed_a_line_at_a_time_and_highlighted() {
    let mut stream = TestStream::new(Some(60), "/repo");
    stream.push("```rust\nfn main() {\n");
    let first = stream.commit();
    assert_eq!(lines_to_plain_strings(&first), vec!["fn main() {"]);
    stream.push("    let x = 1;\n");
    let second = stream.commit();
    assert_eq!(lines_to_plain_strings(&second), vec!["    let x = 1;"]);
    assert!(second[0].spans.len() > 2, "highlighted: {second:?}");
    assert!(stream.tail().is_empty());
}

#[test]
fn the_line_still_arriving_is_previewed_and_its_link_waits_for_its_destination() {
    let (committed, tail) = shown(&["First line.\n", "Hello wor"], Some(40));
    assert_eq!(committed, vec!["First line."]);
    assert_eq!(tail, vec!["Hello wor"]);

    // The label shows as written until its destination closes.
    let (_, tail) = shown(&["See [the docs](https://exa"], Some(60));
    assert_eq!(tail, vec!["See [the docs]"]);
    let (_, tail) = shown(&["See [the docs](https://example.com) now"], Some(60));
    assert_eq!(tail, vec!["See the docs (https://example.com) now"]);

    // CJK arrives a character at a time, wrapped at the width.
    let text = "中文的说明文字需要换行才能放得下";
    let chunks: Vec<String> = text.chars().map(String::from).collect();
    let chunks: Vec<&str> = chunks.iter().map(String::as_str).collect();
    let (_, tail) = shown(&chunks, Some(10));
    assert_eq!(tail.concat(), text);
    assert!(tail.len() > 1);
}

#[test]
fn a_preview_is_not_shown_inside_a_table_or_a_fence() {
    let (_, tail) = shown(&["| A | B |\n|---|---|\n| 1 "], Some(40));
    assert!(!tail.join("\n").contains('1'), "{tail:?}");
    let (_, tail) = shown(&["```rust\nlet partial"], Some(40));
    assert!(tail.is_empty(), "{tail:?}");
}

#[test]
fn a_local_link_line_waits_for_a_colon_line_to_join_it() {
    let (committed, tail) = shown(&["- [binary](/repo/README.md:93)\n"], Some(80));
    assert!(committed.is_empty());
    assert_eq!(tail, vec!["• binary (README.md:93)"]);
    let (committed, _) = shown(
        &[
            "- [binary](/repo/README.md:93)\n",
            "  : the binary.\n",
            "\nAfter.\n",
        ],
        Some(80),
    );
    assert_eq!(committed[0], "• binary (README.md:93): the binary.");
}

#[test]
fn the_tail_starts_the_stream_until_a_row_is_queued() {
    let mut ctrl = stream_controller(Some(40));
    ctrl.push("| A | B |\n");
    let update = ctrl.update(None).expect("the header shows");
    let tail = update.shown.and_then(|rows| rows.0).expect("rows");
    assert!(!tail.is_stream_continuation());
    assert!(lines_to_plain_strings(&tail.display_lines(42))[0].starts_with("• "));
    assert!(ctrl.update(None).is_none(), "nothing changed");

    ctrl.push("|---|---|\n| 1 | 2 |\n\nAfter\n");
    let (cell, _) = ctrl.on_commit_tick_batch(usize::MAX);
    let update = ctrl.update(cell).expect("rows committed");
    assert!(update.committed.is_some());
    assert!(update.shown.is_some_and(|rows| rows.0.is_none()));
}
