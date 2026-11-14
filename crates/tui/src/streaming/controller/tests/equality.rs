// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What a stream commits, however its source arrives, is the full render:
//! the rows committed are never changed afterwards, since they equal the
//! final render's first rows (no repair is asked for), and the rest
//! follows them.

use super::{full_render, lines_to_plain_strings};
use crate::markdown_render::ListSpacing;
use crate::streaming::controller::TestStream;

/// M1's samples (tables, a pipe in prose, ```md fences, a code fence with
/// pipes, quoted and listed tables, links, CJK, bare markers, task lists)
/// and M3's (a long ```md document, a code block, a local link joined by a
/// `:` line, a setext heading).
const SAMPLES: &[&str] = &[
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
    "1. Tight\n\n2.\n   Loose\n",
    "-\n  text\n",
    "> -\n>   quoted text\n",
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
    "```md\n# A long document\n\nIts first paragraph, long enough to wrap at a narrow width.\n\n- an item\n- another item\n\n| Key | Value |\n|---|---|\n| a | b |\n```\n\nAfter the document.\n",
    "```markdown\n# Notes\n\nNo table here, only prose\nthat stays code.\n```\n",
    "Code:\n\n```rust\nfn main() {\n    /* a comment\n       over lines */\n    println!(\"界\");\n}\n```\n\nDone.\n",
    "- [binary](/repo/README.md:93)\n  : the binary.\n- [core](/repo/core/README.md:1)\n  : the core.\n",
    "Title\n=====\n\nSubtitle\n--------\n\nText.\n",
    "Earlier [reference][id].\n\n[id]: https://example.com/reference\n",
];

/// The source in chunks of a few bytes, cut at character boundaries, the
/// cuts spread by a fixed sequence.
pub(super) fn arbitrary_chunks(source: &str, seed: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut step = seed;
    while start < source.len() {
        step = (step * 7 + 3) % 11 + 1;
        let mut end = (start + step).min(source.len());
        while !source.is_char_boundary(end) {
            end += 1;
        }
        chunks.push(&source[start..end]);
        start = end;
    }
    chunks
}

pub(super) fn by_char(source: &str) -> Vec<&str> {
    source
        .char_indices()
        .map(|(index, ch)| &source[index..index + ch.len_utf8()])
        .collect()
}

/// The rows committed after each chunk and at the end, and whether the
/// end asked for a repair.
pub(super) fn stream(
    chunks: &[&str],
    width: Option<usize>,
    spacing: ListSpacing,
) -> (Vec<String>, bool) {
    let mut stream = TestStream::with_spacing(width, "/repo", spacing);
    let mut rows = Vec::new();
    for chunk in chunks {
        stream.push(chunk);
        rows.extend(stream.commit());
    }
    let (remaining, diverged) = stream.finalize_checked();
    rows.extend(remaining);
    (lines_to_plain_strings(&rows), diverged)
}

#[test]
fn every_sample_streams_to_its_full_render_however_it_is_split() {
    for source in SAMPLES {
        let repairs = source.contains("=====") || source.contains("[id]:");
        for width in [None, Some(80), Some(40), Some(20)] {
            let expected = full_render(source, width);
            let splits: Vec<(&str, Vec<&str>)> = vec![
                ("lines", source.split_inclusive('\n').collect()),
                ("chars", by_char(source)),
                ("chunks 1", arbitrary_chunks(source, 1)),
                ("chunks 5", arbitrary_chunks(source, 5)),
                ("whole", vec![source]),
            ];
            for (split, chunks) in splits {
                let (streamed, diverged) = stream(&chunks, width, ListSpacing::AfterMultiline);
                if repairs && diverged {
                    continue;
                }
                assert_eq!(streamed, expected, "{split} at {width:?}:\n{source}");
                assert!(!diverged, "{split} at {width:?}:\n{source}");
            }
        }
    }
}

/// A heading's underline or a definition arriving last changes rows
/// already committed: the stream asks for its repair rather than leave them.
#[test]
fn what_changes_a_committed_row_asks_for_a_repair() {
    for source in [
        "Title\n=====\n\nText.\n",
        "Earlier [reference][id].\n\n[id]: https://example.com/reference\n",
    ] {
        let chunks: Vec<&str> = source.split_inclusive('\n').collect();
        let (streamed, diverged) = stream(&chunks, Some(40), ListSpacing::AfterMultiline);
        assert_ne!(streamed, full_render(source, Some(40)));
        assert!(diverged, "{source}");
    }
}

/// Fullscreen streams its lists compact: the stream equals the compact
/// render, which the consolidated cell then spaces uniformly.
#[test]
fn compact_streams_equal_the_compact_render() {
    for source in SAMPLES
        .iter()
        .filter(|source| !source.contains("=====") && !source.contains("[id]:"))
    {
        for width in [Some(80), Some(24)] {
            let chunks: Vec<&str> = source.split_inclusive('\n').collect();
            let (streamed, _) = stream(&chunks, width, ListSpacing::Compact);
            let expected = crate::markdown::render_streaming_markdown_agent(
                source,
                width,
                Some("/repo"),
                ListSpacing::Compact,
            );
            assert_eq!(
                streamed,
                lines_to_plain_strings(&crate::terminal_hyperlinks::visible_lines(expected.lines)),
                "at {width:?}:\n{source}"
            );
        }
    }
}

/// With `tui.rendering.tables` or `.lists` off, a stream equals the full
/// render too: a table, and a Markdown fence holding one, are shown as
/// written, the fence decided when it closes.
#[test]
fn every_sample_streams_to_its_full_render_with_tables_or_lists_off() {
    for (tables, lists) in [(false, true), (true, false), (false, false)] {
        crate::markdown_render::preferences::set(hanafubuki_agent::AgentTuiRendering {
            tables,
            lists,
            ..Default::default()
        });
        for source in SAMPLES
            .iter()
            .filter(|source| !source.contains("=====") && !source.contains("[id]:"))
        {
            let expected = full_render(source, Some(40));
            for chunks in [source.split_inclusive('\n').collect(), by_char(source)] {
                let (streamed, diverged) = stream(&chunks, Some(40), ListSpacing::AfterMultiline);
                assert_eq!(
                    streamed, expected,
                    "tables {tables}, lists {lists}:\n{source}"
                );
                assert!(!diverged, "tables {tables}, lists {lists}:\n{source}");
            }
        }
    }
}
