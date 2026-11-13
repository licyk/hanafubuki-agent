// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use ratatui::style::Color;
use ratatui::text::Line;

use super::MarkdownStreamCollector;
use crate::streaming::controller::TestStream;

/// What a stream of `deltas` commits, all it queued after each delta, and
/// the rest at its end when `finalize`.
fn simulate_stream_markdown_for_tests(deltas: &[&str], finalize: bool) -> Vec<Line<'static>> {
    let mut stream = TestStream::new(None, "");
    let mut out = Vec::new();
    for delta in deltas {
        stream.push(delta);
        out.extend(stream.commit());
    }
    if finalize {
        out.extend(stream.finalize());
    }
    out
}

#[test]
fn the_source_is_committed_a_complete_line_at_a_time() {
    let mut collector = MarkdownStreamCollector::new();
    collector.push_delta("Hello, wor");
    assert_eq!(collector.commit_complete_source(), None);
    collector.push_delta(
        "ld
Next",
    );
    assert_eq!(collector.commit_complete_source(), Some(0..13));
    assert_eq!(collector.committed_source(), "Hello, world\n");
    assert_eq!(collector.pending_source(), "Next");
    assert_eq!(collector.commit_complete_source(), None);
    assert_eq!(collector.finalize_and_take_source(), "Hello, world\nNext");
    assert_eq!(collector.committed_source(), "");
}

#[test]
fn finalizing_empty_collector_returns_empty_source() {
    assert_eq!(
        MarkdownStreamCollector::new().finalize_and_take_source(),
        ""
    );
}

#[test]
fn no_commit_until_newline() {
    let mut c = TestStream::new(None, "");
    c.push("Hello, world");
    let out = c.commit();
    assert!(out.is_empty(), "should not commit without newline");
    c.push("!\n");
    let out2 = c.commit();
    assert_eq!(out2.len(), 1, "one completed line after newline");
}

#[test]
fn finalize_commits_partial_line() {
    let mut c = TestStream::new(None, "");
    c.push("Line without newline");
    let out = c.finalize();
    assert_eq!(out.len(), 1);
}

#[test]
fn e2e_stream_blockquote_simple_is_green() {
    let out = simulate_stream_markdown_for_tests(&["> Hello\n"], true);
    assert_eq!(out.len(), 1);
    let l = &out[0];
    assert_eq!(
        l.style.fg,
        Some(Color::Green),
        "expected blockquote line fg green, got {:?}",
        l.style.fg
    );
}

#[test]
fn e2e_stream_blockquote_nested_is_green() {
    let out = simulate_stream_markdown_for_tests(&["> Level 1\n>> Level 2\n"], true);
    // Filter out any blank lines that may be inserted at paragraph starts.
    let non_blank: Vec<_> = out
        .into_iter()
        .filter(|l| {
            let s = l
                .spans
                .iter()
                .map(|sp| sp.content.clone())
                .collect::<Vec<_>>()
                .join("");
            let t = s.trim();
            // Ignore quote-only blank lines like ">" inserted at paragraph boundaries.
            !(t.is_empty() || t == ">")
        })
        .collect();
    assert_eq!(non_blank.len(), 2);
    assert_eq!(non_blank[0].style.fg, Some(Color::Green));
    assert_eq!(non_blank[1].style.fg, Some(Color::Green));
}

#[test]
fn e2e_stream_blockquote_with_list_items_is_green() {
    let out = simulate_stream_markdown_for_tests(&["> - item 1\n> - item 2\n"], true);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].style.fg, Some(Color::Green));
    assert_eq!(out[1].style.fg, Some(Color::Green));
}

#[test]
fn e2e_stream_nested_mixed_lists_ordered_marker_is_light_blue() {
    let md = [
        "1. First\n",
        "   - Second level\n",
        "     1. Third level (ordered)\n",
        "        - Fourth level (bullet)\n",
        "          - Fifth level to test indent consistency\n",
    ];
    let out = simulate_stream_markdown_for_tests(&md, true);
    // Find the line that contains the third-level ordered text
    let find_idx = out.iter().position(|l| {
        l.spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<String>()
            .contains("Third level (ordered)")
    });
    let idx = find_idx.expect("expected third-level ordered line");
    let line = &out[idx];
    // Expect at least one span on this line to be styled light blue
    let has_light_blue = line
        .spans
        .iter()
        .any(|s| s.style.fg == Some(ratatui::style::Color::LightBlue));
    assert!(
        has_light_blue,
        "expected an ordered-list marker span with light blue fg on: {line:?}"
    );
}

#[test]
fn e2e_stream_blockquote_wrap_preserves_green_style() {
    let long = "> This is a very long quoted line that should wrap across multiple columns to verify style preservation.";
    let out = simulate_stream_markdown_for_tests(&[long, "\n"], true);
    // Wrap to a narrow width to force multiple output lines.
    let wrapped = crate::wrapping::word_wrap_lines(out.iter(), crate::wrapping::RtOptions::new(24));
    // Filter out purely blank lines
    let non_blank: Vec<_> = wrapped
        .into_iter()
        .filter(|l| {
            let s = l
                .spans
                .iter()
                .map(|sp| sp.content.clone())
                .collect::<Vec<_>>()
                .join("");
            !s.trim().is_empty()
        })
        .collect();
    assert!(
        non_blank.len() >= 2,
        "expected wrapped blockquote to span multiple lines"
    );
    for (i, l) in non_blank.iter().enumerate() {
        assert_eq!(
            l.spans[0].style.fg,
            Some(Color::Green),
            "wrapped line {} should preserve green style, got {:?}",
            i,
            l.spans[0].style.fg
        );
    }
}

#[test]
fn heading_starts_on_new_line_when_following_paragraph() {
    // Stream a paragraph line, then a heading on the next line.
    // Expect two distinct rendered lines: "Hello." and "Heading".
    let mut c = TestStream::new(None, "");
    c.push("Hello.\n");
    let out1 = c.commit();
    let s1: Vec<String> = out1
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    assert_eq!(
        out1.len(),
        1,
        "first commit should contain only the paragraph line, got {}: {:?}",
        out1.len(),
        s1
    );

    c.push("## Heading\n");
    let out2 = c.commit();
    let s2: Vec<String> = out2
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    assert_eq!(
        s2,
        vec!["", "## Heading"],
        "expected a blank separator then the heading line"
    );

    let line_to_string = |l: &ratatui::text::Line<'_>| -> String {
        l.spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<Vec<_>>()
            .join("")
    };

    assert_eq!(line_to_string(&out1[0]), "Hello.");
    assert_eq!(line_to_string(&out2[1]), "## Heading");
}

#[test]
fn heading_not_inlined_when_split_across_chunks() {
    // Paragraph without trailing newline, then a chunk that starts with the newline
    // and the heading text, then a final newline. The collector should first commit
    // only the paragraph line, and later commit the heading as its own line.
    let mut c = TestStream::new(None, "");
    c.push("Sounds good!");
    // No commit yet
    assert!(c.commit().is_empty());

    // Introduce the newline that completes the paragraph and the start of the heading.
    c.push("\n## Adding Bird subcommand");
    let out1 = c.commit();
    let s1: Vec<String> = out1
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    assert_eq!(
        s1,
        vec!["Sounds good!"],
        "expected paragraph followed by blank separator before heading chunk"
    );

    // Now finish the heading line with the trailing newline.
    c.push("\n");
    let out2 = c.commit();
    let s2: Vec<String> = out2
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    assert_eq!(
        s2,
        vec!["", "## Adding Bird subcommand"],
        "expected the heading line only on the final commit"
    );

    // Sanity check raw markdown rendering for a simple line does not produce spurious extras.
    let mut rendered: Vec<ratatui::text::Line<'static>> = Vec::new();
    crate::markdown::append_markdown_agent("Hello.\n", None, None, &mut rendered);
    let rendered_strings: Vec<String> = rendered
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    assert_eq!(
        rendered_strings,
        vec!["Hello."],
        "unexpected markdown lines: {rendered_strings:?}"
    );
}

fn lines_to_plain_strings(lines: &[ratatui::text::Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect()
}

#[test]
fn lists_and_fences_commit_without_duplication() {
    // List case
    assert_streamed_equals_full(&["- a\n- ", "b\n- c\n"]);

    // Fenced code case: stream in small chunks
    assert_streamed_equals_full(&["```", "\nco", "de 1\ncode 2\n", "```\n"]);
}

#[test]
fn utf8_boundary_safety_and_wide_chars() {
    // Emoji (wide), CJK, control char, digit + combining macron sequences
    let input = "🙂🙂🙂\n汉字漢字\nA\u{0003}0\u{0304}\n";
    let deltas = vec![
        "🙂",
        "🙂",
        "🙂\n汉",
        "字漢",
        "字\nA",
        "\u{0003}",
        "0",
        "\u{0304}",
        "\n",
    ];

    let streamed = simulate_stream_markdown_for_tests(&deltas, true);
    let streamed_str = lines_to_plain_strings(&streamed);

    let mut rendered_all: Vec<ratatui::text::Line<'static>> = Vec::new();
    crate::markdown::append_markdown_agent(input, None, None, &mut rendered_all);
    let rendered_all_str = lines_to_plain_strings(&rendered_all);

    assert_eq!(
        streamed_str, rendered_all_str,
        "utf8/wide-char streaming should equal full render without duplication or truncation"
    );
}

#[test]
fn e2e_stream_deep_nested_third_level_marker_is_light_blue() {
    let md = "1. First\n   - Second level\n     1. Third level (ordered)\n        - Fourth level (bullet)\n          - Fifth level to test indent consistency\n";
    let streamed = simulate_stream_markdown_for_tests(&[md], true);
    let streamed_strs = lines_to_plain_strings(&streamed);

    // Locate the third-level line in the streamed output; avoid relying on exact indent.
    let target_suffix = "1. Third level (ordered)";
    let mut found = None;
    for line in &streamed {
        let s: String = line.spans.iter().map(|sp| sp.content.clone()).collect();
        if s.contains(target_suffix) {
            found = Some(line.clone());
            break;
        }
    }
    let line = found.unwrap_or_else(|| {
        panic!("expected to find the third-level ordered list line; got: {streamed_strs:?}")
    });

    // The marker (including indent and "1.") is expected to be in the first span
    // and colored LightBlue; following content should be default color.
    assert!(
        !line.spans.is_empty(),
        "expected non-empty spans for the third-level line"
    );
    let marker_span = &line.spans[0];
    assert_eq!(
        marker_span.style.fg,
        Some(Color::LightBlue),
        "expected LightBlue 3rd-level ordered marker, got {:?}",
        marker_span.style.fg
    );
    // Find the first non-empty non-space content span and verify it is default color.
    let mut content_fg = None;
    for sp in &line.spans[1..] {
        let t = sp.content.trim();
        if !t.is_empty() {
            content_fg = Some(sp.style.fg);
            break;
        }
    }
    assert_eq!(
        content_fg.flatten(),
        None,
        "expected default color for 3rd-level content, got {content_fg:?}"
    );
}

#[test]
fn empty_fenced_block_is_dropped_and_separator_preserved_before_heading() {
    // An empty fenced code block followed by a heading should not render the fence,
    // but should preserve a blank separator line so the heading starts on a new line.
    let deltas = vec!["```bash\n```\n", "## Heading\n"]; // empty block and close in same commit
    let streamed = simulate_stream_markdown_for_tests(&deltas, true);
    let texts = lines_to_plain_strings(&streamed);
    assert!(
        texts.iter().all(|s| !s.contains("```")),
        "no fence markers expected: {texts:?}"
    );
    // Expect the heading and no fence markers. A blank separator may or may not be rendered at start.
    assert!(
        texts.iter().any(|s| s == "## Heading"),
        "expected heading line: {texts:?}"
    );
}

#[test]
fn paragraph_then_empty_fence_then_heading_keeps_heading_on_new_line() {
    let deltas = vec!["Para.\n", "```\n```\n", "## Title\n"]; // empty fence block in one commit
    let streamed = simulate_stream_markdown_for_tests(&deltas, true);
    let texts = lines_to_plain_strings(&streamed);
    let para_idx = match texts.iter().position(|s| s == "Para.") {
        Some(i) => i,
        None => panic!("para present"),
    };
    let head_idx = match texts.iter().position(|s| s == "## Title") {
        Some(i) => i,
        None => panic!("heading present"),
    };
    assert!(
        head_idx > para_idx,
        "heading should not merge with paragraph: {texts:?}"
    );
}

#[test]
fn loose_list_with_split_dashes_matches_full_render() {
    // Minimized failing sequence discovered by the helper: two chunks
    // that still reproduce the mismatch.
    let deltas = vec!["- item.\n\n", "-"];

    let streamed = simulate_stream_markdown_for_tests(&deltas, true);
    let streamed_strs = lines_to_plain_strings(&streamed);

    let full: String = deltas.iter().copied().collect();
    let mut rendered_all: Vec<ratatui::text::Line<'static>> = Vec::new();
    crate::markdown::append_markdown_agent(&full, None, None, &mut rendered_all);
    let rendered_all_strs = lines_to_plain_strings(&rendered_all);

    assert_eq!(
        streamed_strs, rendered_all_strs,
        "streamed output should match full render without dangling '-' lines"
    );
}

#[test]
fn loose_vs_tight_list_items_streaming_matches_full() {
    // Deltas extracted from the session log around 2025-08-27T00:33:18.216Z
    let deltas = vec![
        "\n\n",
        "Loose",
        " vs",
        ".",
        " tight",
        " list",
        " items",
        ":\n",
        "1",
        ".",
        " Tight",
        " item",
        "\n",
        "2",
        ".",
        " Another",
        " tight",
        " item",
        "\n\n",
        "1",
        ".",
        " Loose",
        " item",
        " with",
        " its",
        " own",
        " paragraph",
        ".\n\n",
        "  ",
        " This",
        " paragraph",
        " belongs",
        " to",
        " the",
        " same",
        " list",
        " item",
        ".\n\n",
        "2",
        ".",
        " Second",
        " loose",
        " item",
        " with",
        " a",
        " nested",
        " list",
        " after",
        " a",
        " blank",
        " line",
        ".\n\n",
        "  ",
        " -",
        " Nested",
        " bullet",
        " under",
        " a",
        " loose",
        " item",
        "\n",
        "  ",
        " -",
        " Another",
        " nested",
        " bullet",
        "\n\n",
    ];

    let streamed = simulate_stream_markdown_for_tests(&deltas, true);
    let streamed_strs = lines_to_plain_strings(&streamed);

    // Compute a full render for diagnostics only.
    let full: String = deltas.iter().copied().collect();
    let mut rendered_all: Vec<ratatui::text::Line<'static>> = Vec::new();
    crate::markdown::append_markdown_agent(&full, None, None, &mut rendered_all);

    // Also assert exact expected plain strings for clarity.
    let expected = vec![
        "Loose vs. tight list items:".to_string(),
        "".to_string(),
        "1. Tight item".to_string(),
        "2. Another tight item".to_string(),
        "3. Loose item with its own paragraph.".to_string(),
        "".to_string(),
        "   This paragraph belongs to the same list item.".to_string(),
        "".to_string(),
        "4. Second loose item with a nested list after a blank line.".to_string(),
        "    • Nested bullet under a loose item".to_string(),
        "    • Another nested bullet".to_string(),
    ];
    assert_eq!(
        streamed_strs, expected,
        "expected exact rendered lines for loose/tight section"
    );
}

// Targeted tests derived from fuzz findings. Each asserts streamed == full render.
fn assert_streamed_equals_full(deltas: &[&str]) {
    let streamed = simulate_stream_markdown_for_tests(deltas, true);
    let streamed_strs = lines_to_plain_strings(&streamed);
    let full: String = deltas.iter().copied().collect();
    let mut rendered: Vec<ratatui::text::Line<'static>> = Vec::new();
    crate::markdown::append_markdown_agent(&full, None, None, &mut rendered);
    let rendered_strs = lines_to_plain_strings(&rendered);
    assert_eq!(streamed_strs, rendered_strs, "full:\n---\n{full}\n---");
}

#[test]
fn fuzz_class_bullet_duplication_variant_1() {
    assert_streamed_equals_full(&["aph.\n- let one\n- bull", "et two\n\n  second paragraph \n"])
}

#[test]
fn fuzz_class_bullet_duplication_variant_2() {
    assert_streamed_equals_full(&[
        "- e\n  c",
        "e\n- bullet two\n\n  second paragraph in bullet two\n",
    ])
}

#[test]
fn streaming_html_block_then_text_matches_full() {
    assert_streamed_equals_full(&["HTML block:\n", "<div>inline block</div>\n", "more stuff\n"])
}

/// What M1's constructs stream to, line by line at a width, is what the
/// whole answer renders to: a table's rows wait until it has ended, a
/// ```md fence until it is unwrapped or not.
#[test]
fn tables_task_lists_and_links_stream_as_they_render() {
    let samples = [
        "Intro\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| a longer cell that wraps | x |\n\nAfter the table.\n",
        "Intro line\n| Key | Notes |\n| --- | --- |\n| firstlongid | A readable explanatory sentence for this row. |\n| secondlongid | Another readable explanatory sentence. |\n",
        "| A | B |\n|---|---|\n| 1 | 2 |\nTrailing prose line\n\nNext paragraph\n",
        "Text with a | pipe that is no table.\nMore text.\n",
        "```md\n| A | B |\n|---|---|\n| 1 | 2 |\n```\n\nAfter the fence.\n",
        "```md\n**Summary**\n\n| A | B |\n|---|---|\n| 1 | 2 |\n```\nAfter the fence.\n",
        "```markdown\n**not a table**\n```\nText right after.\n",
        "```rust\nlet x = a | b;\nlet y = c | d;\n```\n",
        "1. Results:\n\n   | Case | Result |\n   |---|---|\n   | first | passed |\n\n2. Next item\n",
        "> | A | B |\n> |---|---|\n> | 1 | 2 |\n>\n> quoted after\n",
        "- [ ] Unchecked task with enough words to wrap\n- [x] Checked\n  - [ ] Nested\n- Ordinary\n",
        "See [docs](https://example.com/docs) and [lib](/repo/src/lib.rs:12).\n- [binary](/repo/README.md:93): core\n",
        "| 名前 | Notes |\n|:---|---:|\n| 東京タワー | 中文的说明文字，需要换行才能放得下 |\n| 🍣 |  |\n",
        // An item whose marker is alone on its line, its text on the next.
        "1. Tight\n\n2.\n   Loose\n",
        "-\n  text\n",
        "> -\n>   quoted text\n",
        // Codex's task-list samples.
        "- [ ] Unchecked\n- [x] Checked\n- [X] Uppercase\n- Ordinary\n",
        "- [ ] Parent task with enough words to wrap\n  - [x] Nested task\n  - Ordinary child\n- [x] Done\n",
        "9. [ ] Numbered task with enough words to wrap\n10. [x] Done\n11. Ordinary\n",
        "> - [ ] Quoted task with enough words to wrap\n> - [x] Done\n",
        "- [ ] First paragraph\n\n  Second paragraph\n\n- [x] Next task\n",
        "- [ ]\n- [x]\n",
        "- [x] **Bold**, *italic*, and `code`\n\n`- [x] inline`\n\n```text\n- [x] fenced\n```\n\n[ ] Plain text\n\n- \\[x] Escaped\n",
        "- [ ] \n  - child\n\n- [x] # title\n\n- [x] > quote\n\n- [x] ---\n",
        "-     [x] code, not a task\n",
        "- [x] <b>done</b>\n\n- Parent\n  - [x] \n\n  Continued paragraph\n",
        "- [x] <div>done</div>\n",
        "- [x] \n  <div>done</div>\n",
        "> - Parent\n>   - [x] \n>\n>   Continued paragraph\n",
    ];
    for source in samples {
        for width in [None, Some(80), Some(40), Some(20)] {
            let mut collector = TestStream::new(width, "/repo");
            let mut streamed = Vec::new();
            for line in source.split_inclusive('\n') {
                collector.push(line);
                streamed.extend(collector.commit());
            }
            streamed.extend(collector.finalize());
            let mut rendered = Vec::new();
            crate::markdown::append_markdown_agent(source, width, Some("/repo"), &mut rendered);
            assert_eq!(
                lines_to_plain_strings(&streamed),
                lines_to_plain_strings(&rendered),
                "at {width:?}:\n{source}"
            );
        }
    }
}

#[test]
fn a_table_is_held_until_a_blank_line_ends_it() {
    let mut collector = TestStream::new(None, "");
    collector.push("Intro\n\n| A | B |\n");
    assert_eq!(lines_to_plain_strings(&collector.commit()), vec!["Intro"]);
    collector.push("|---|---|\n| 1 | 2 |\n");
    assert!(collector.commit().is_empty());
    collector.push("\nAfter\n");
    assert_eq!(
        lines_to_plain_strings(&collector.commit()),
        vec!["", " A      B", "━━━━━  ━━━━━", " 1      2", "", "After"]
    );
}
