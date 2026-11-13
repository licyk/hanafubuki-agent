// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use hanafubuki_agent::AgentTuiRendering;

use super::StreamingRender;
use crate::markdown::render_streaming_markdown_agent;
use crate::markdown_render::ListSpacing;
use crate::render::highlight::MAX_HIGHLIGHT_LINE_BYTES;

const CWD: &str = "/repo";

fn new_render() -> StreamingRender {
    StreamingRender::new(render_streaming_markdown_agent, ListSpacing::AfterMultiline)
}

fn append(render: &mut StreamingRender, source: &mut String, chunk: &str, width: Option<usize>) {
    source.push_str(chunk);
    render.append(source, chunk, width, CWD);
}

fn append_and_assert_matches_full(
    render: &mut StreamingRender,
    source: &mut String,
    chunk: &str,
    width: Option<usize>,
) {
    append(render, source, chunk, width);
    assert_eq!(
        render.lines,
        render.render_source(source, width, CWD).lines,
        "incremental render diverged after chunk {chunk:?}",
    );
}

fn lines_to_plain_strings(lines: &[ratatui::text::Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

fn assert_stream_matches_full_render(
    chunks: &[&str],
    width: Option<usize>,
) -> (String, StreamingRender) {
    let mut source = String::new();
    let mut render = new_render();
    for chunk in chunks {
        append_and_assert_matches_full(&mut render, &mut source, chunk, width);
    }
    (source, render)
}

#[test]
fn incremental_render_keeps_final_block_mutable_and_matches_full_render() {
    let chunks = [
        "# Heading\n",
        "\n",
        "First paragraph with a [link](https://example.com).\n",
        "continued on the next line.\n\n",
        "1. First item\n",
        "2. Second item\n\n",
        "> Quoted paragraph\n\n",
        "```rust\n",
        "fn main() {}\n",
        "```\n\n",
        "| Key | Value |\n",
        "| --- | --- |\n",
        "| alpha | beta |\n",
    ];
    let (source, render) = assert_stream_matches_full_render(&chunks, Some(48));

    assert!(render.stable_source_len > 0);
    assert!(render.stable_source_len < source.len());
}

#[test]
fn growing_single_top_level_blocks_render_in_one_pass() {
    let streams: &[&[&str]] = &[
        &[
            "A paragraph that keeps growing\n",
            "without a blank line between chunks.\n",
            "It stays one top-level block.\n",
        ],
        &[
            "| Key | Value |\n",
            "| --- | --- |\n",
            "| alpha | beta |\n",
            "| gamma | delta |\n",
        ],
    ];
    let width = Some(80);
    for chunks in streams {
        let mut source = String::new();
        let mut render = new_render();
        for chunk in *chunks {
            append_and_assert_matches_full(&mut render, &mut source, chunk, width);
            let pending =
                render_streaming_markdown_agent(&source, width, Some(CWD), ListSpacing::default());
            assert_eq!(pending.last_top_level_block_start, None);
            assert_eq!(render.lines, pending.lines);
            assert_eq!(render.stable_source_len, 0);
        }
    }
}

#[test]
fn compact_list_spacing_keeps_incremental_and_full_renders_aligned() {
    let mut render = StreamingRender::new(render_streaming_markdown_agent, ListSpacing::Compact);
    let mut source = String::new();
    for chunk in [
        "- Short\n",
        "- A sibling with enough words to wrap across rows\n",
        "- Last\n",
        "\nAfter the list.\n\n",
        "| Name | Value |\n| --- | --- |\n| A | B |\n",
        "\n- Nested\n  - One\n  - Two\n- End\n",
    ] {
        append(&mut render, &mut source, chunk, Some(24));
        let full =
            render_streaming_markdown_agent(&source, Some(24), Some(CWD), ListSpacing::Compact);
        assert_eq!(render.lines, full.lines);
    }
    for width in [80, 24] {
        render.recompute(&source, Some(width), CWD);
        let full =
            render_streaming_markdown_agent(&source, Some(width), Some(CWD), ListSpacing::Compact);
        assert_eq!(render.lines, full.lines);
    }
}

#[test]
fn reference_link_definition_recomputes_earlier_and_later_blocks() {
    let streams: &[&[&str]] = &[
        &[
            "Earlier [reference][id].\n\n",
            "An unrelated paragraph.\n\n",
            "[id]: https://example.com/reference\n",
            "\n",
            "Later [reference][id].\n",
        ],
        &[
            "Earlier [reference][id].\n\n",
            "Another paragraph.\n\n",
            "```markdown\n",
            "| Key | Value |\n",
            "| --- | --- |\n",
            "| alpha | beta |\n",
            "\n",
            "[id]: https://example.com/reference\n",
            "```\n",
        ],
    ];
    for chunks in streams {
        let (_, render) = assert_stream_matches_full_render(chunks, Some(80));
        assert!(render.has_reference_link_definition);
    }
}

#[test]
fn incremental_render_does_not_add_blank_line_before_html_block() {
    let streams: &[&[&str]] = &[
        &[
            "Paragraph.\n\n",
            "<div>x</div>\n\n",
            "Following paragraph.\n",
        ],
        &["Paragraph.\n\n<div>x</div>\n\n", "Following paragraph.\n"],
    ];
    for chunks in streams {
        let (_, render) = assert_stream_matches_full_render(chunks, Some(80));
        assert_eq!(
            lines_to_plain_strings(&render.lines),
            vec!["Paragraph.", "<div>x</div>", "", "Following paragraph."],
        );
    }
}

#[test]
fn incremental_render_preserves_heading_and_normalized_html_seams() {
    let streams: &[&[&str]] = &[
        &["Paragraph.\n\n# Heading\n\n", "Next paragraph.\n"],
        &[
            "Paragraph.\n\n",
            "<div>x</div>\n\n",
            "```markdown\n",
            "| Key | Value |\n",
            "| --- | --- |\n",
            "| alpha | beta |\n",
            "\n",
            "Following paragraph.\n",
            "```\n",
        ],
    ];
    for chunks in streams {
        assert_stream_matches_full_render(chunks, Some(80));
    }
}

#[test]
fn paragraphs_after_unwrapped_table_fence_advance_stable_source() {
    let width = Some(80);
    let (mut source, mut render) = assert_stream_matches_full_render(
        &[
            "```markdown\n",
            "| Key | Value |\n",
            "| --- | --- |\n",
            "| alpha | beta |\n",
            "```\n\n",
        ],
        width,
    );

    let mut previous_stable_source_len = render.stable_source_len;
    for block in ["<div>Post-fence HTML.</div>\n\n", "First paragraph.\n\n"] {
        append_and_assert_matches_full(&mut render, &mut source, block, width);
        assert!(render.stable_source_len > previous_stable_source_len);
        previous_stable_source_len = render.stable_source_len;
    }
}

#[test]
fn an_open_markdown_fence_stays_mutable_whatever_the_tables_preference() {
    for tables in [true, false] {
        crate::markdown_render::preferences::set(AgentTuiRendering {
            tables,
            lists: true,
        });
        let (source, render) = assert_stream_matches_full_render(
            &["Intro.\n\n", "```md\n", "# Title\n", "Prose inside.\n"],
            Some(40),
        );
        assert_eq!(
            render.mutable_fence_start,
            Some("Intro.\n\n".len()),
            "{source:?}"
        );
    }
}

#[test]
fn growing_code_fences_preserve_styles_unicode_links_and_blank_lines() {
    let (_, render) = assert_stream_matches_full_render(
        &[
            "Before [link](https://example.com).\n\n```rust,no_run\n",
            "fn main() {\n",
            "    /* a multiline\n",
            "       comment */\n\n",
            "    println!(\"界 e\u{301} https://example.com\");\n",
            "}\n",
            "```\n\n~~~~python\n",
            "text = \"\"\"a multiline\n",
            "string\"\"\"\n",
            "~~~~\n",
        ],
        Some(18),
    );
    assert!(render.open_code_fence.is_none());
}

#[test]
fn long_open_fence_retains_its_rendered_prefix() {
    let mut source = String::new();
    let mut render = new_render();
    append_and_assert_matches_full(&mut render, &mut source, "```rust\nfn main() {\n", Some(80));
    let first_line = render.lines[0].spans[1].content.as_ptr();
    for index in 0..512 {
        append(
            &mut render,
            &mut source,
            &format!("    let value_{index} = {index};\n"),
            Some(80),
        );
        assert!(render.open_code_fence.is_some());
    }
    assert_eq!(render.lines[0].spans[1].content.as_ptr(), first_line);
    append_and_assert_matches_full(&mut render, &mut source, "}\n```\n", Some(80));
}

#[test]
fn ambiguous_and_normalized_fences_use_canonical_semantics() {
    let streams: &[&[&str]] = &[
        &["```rust\n", "x\n", "``", "`\n"],
        &["```rust\n", "x\n", "```\t\n", "after\n"],
        &["```rust\n", "x\n", "  ````  \n", "after\n"],
        &["~~~rust\n", "x\n", "~~~\u{a0}\n", "after\n"],
        &["```rust\r\n", "let value = 'é';\r\n", "```\r\n"],
        &["```rust\n", "let value = '\0';\n", "```\n"],
        &["```r&#117;st,no_run\n", "fn main() {}\n", "```\n"],
        &[" ```rust\n", "  fn main() {}\n", " ```\n"],
        &["```unknown-language\n", "first\n", "second\n", "```\n"],
        &[
            "```markdown\n| A | B |\n",
            "| --- | --- |\n",
            "| 1 | 2 |\n```\n\n```rust\nfn first() {}\n",
            "fn second() {}\n",
            "```\n",
        ],
    ];
    for chunks in streams {
        assert_stream_matches_full_render(chunks, Some(40));
    }
    for indent in 0..=3 {
        let closing = format!("{}~~~~~\t\n", " ".repeat(indent));
        assert_stream_matches_full_render(&["~~~~rust\n", "x\n", &closing, "after\n"], Some(40));
    }
}

#[test]
fn crossing_a_highlight_limit_removes_previously_highlighted_colors() {
    let mut source = String::new();
    let mut render = new_render();
    for chunk in [
        "```rust\nlet first = 1;\n".to_string(),
        format!("{}\n", "x".repeat(MAX_HIGHLIGHT_LINE_BYTES + 1)),
        "let second = 2;\n".to_string(),
        "```\n".to_string(),
    ] {
        append_and_assert_matches_full(&mut render, &mut source, &chunk, Some(80));
    }
}

#[test]
fn recomputing_after_resize_drops_fence_state() {
    let (mut source, mut render) =
        assert_stream_matches_full_render(&["```rust\nfn first() {}\n"], Some(80));
    assert!(render.open_code_fence.is_some());
    render.recompute(&source, Some(20), CWD);
    assert!(render.open_code_fence.is_none());
    append_and_assert_matches_full(&mut render, &mut source, "fn second() {}\n", Some(20));
    assert!(render.open_code_fence.is_some());
}
