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
            lines_to_plain_strings(&crate::terminal_hyperlinks::visible_lines(
                render.lines.clone()
            )),
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
            ..Default::default()
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
    let first_line = render.lines[0].line.spans[1].content.as_ptr();
    for index in 0..512 {
        append(
            &mut render,
            &mut source,
            &format!("    let value_{index} = {index};\n"),
            Some(80),
        );
        assert!(render.open_code_fence.is_some());
    }
    assert_eq!(render.lines[0].line.spans[1].content.as_ptr(), first_line);
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

#[test]
fn unicode_math_inline_stream_preserves_display_exclusion_boundaries() {
    for (open, close) in [("$$", "$$"), ("\\[", "\\]")] {
        let mut render = new_render();
        let mut source = String::new();
        for chunk in [open, "\n\nx^2\n\n", close, "\n\n", "After $\\alpha$.\n"] {
            append_and_assert_matches_full(&mut render, &mut source, chunk, Some(40));
            for width in [16, 40] {
                let mut resized = new_render();
                resized.recompute(&source, Some(width), CWD);
                assert_eq!(
                    resized.lines,
                    resized.render_source(&source, Some(width), CWD).lines
                );
            }
        }
    }
}

#[test]
fn unicode_math_rejected_display_keeps_its_closer_and_following_text() {
    for (open, close) in [("$$", "$$"), ("\\[", "\\]")] {
        for (prefix, body) in [
            ("", "x".repeat(5000)),
            ("", "`x`".to_owned()),
            ("", format!("`{close}\nx`")),
            ("", "[x](https://example.com)".to_owned()),
            ("Equation: ", "x^2".to_owned()),
        ] {
            let mut render = new_render();
            let mut source = String::new();
            for chunk in [
                format!("{prefix}{open}\n\n"),
                format!("{body}\n\n"),
                format!("{close}\n\n"),
                "After $\\alpha$.\n\n".to_owned(),
                "$$\\beta$$\n".to_owned(),
            ] {
                append(&mut render, &mut source, &chunk, Some(40));
                let expected = render.render_source(&source, Some(40), CWD);
                assert_eq!(
                    (&render.lines, render.pending_math_start),
                    (&expected.lines, expected.pending_math_start)
                );
                if source.len() > 4096 || !prefix.is_empty() {
                    assert_eq!(render.pending_math_start, None);
                }
            }
            let text = lines_to_plain_strings(&crate::terminal_hyperlinks::visible_lines(
                render.lines.clone(),
            ));
            assert!(
                text.iter().any(|line| line.contains("After α.")),
                "{text:?}"
            );
            assert!(text.iter().any(|line| line.contains('β')), "{text:?}");
            assert_eq!(render.pending_math_start, None);
        }
    }
}

/// A rejected opener (a shell's PID, a display after text) keeps only a
/// bounded part of the stream mutable while it looks for its closer.
#[test]
fn rejected_math_openers_allow_bounded_incremental_rendering() {
    for (open, close) in [
        ("echo $$", "$$"),
        ("Equation: \\[", "\\]"),
        (r"\[label\]\*", "\\]"),
    ] {
        let (mut source, mut render) =
            assert_stream_matches_full_render(&[&format!("{open}\n\n")], Some(80));
        let distant_closer = format!("{close}\n\nAfter $\\alpha$.\n\n");
        for chunk in std::iter::repeat_n(
            "An ordinary paragraph that must not retain the entire response.\n\n",
            160,
        )
        .chain(std::iter::once(distant_closer.as_str()))
        {
            append_and_assert_matches_full(&mut render, &mut source, chunk, Some(80));
            assert!(source.len() - render.stable_source_len < 4200);
        }
    }
}

/// With each of `tui.rendering.math` and `.tables` on and off, the
/// incremental render equals the full one at every line. With math off an
/// open display is still held: the masker still finds it, and a code block
/// later inside it can still reject it.
#[test]
fn rendering_preferences_match_incremental_and_full_renders() {
    let source = concat!(
        "Before.\n\n",
        "```markdown\n| A | B |\n|---|---|\n| `a` | \\(x_1\\) |\n```\n\n",
        "```mermaid\nflowchart LR\nA --> B\n```\n\n",
        "Inline \\(\\alpha_1\\) and $x^2$.\n\n\\[\n\\frac{a}{b}\n\\]\n\n",
        "| Name | Value |\n|---|---|\n| **a** | $x^2$ |\n\n",
        "Prose after the table.\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\n",
        "| **Name** | Value |\n|---|---|\n| `a` | b |\n\nAfter.\n"
    );
    for mermaid in [true, false] {
        for math in [true, false] {
            for tables in [true, false] {
                crate::markdown_render::preferences::set(AgentTuiRendering {
                    mermaid,
                    math,
                    tables,
                    ..Default::default()
                });
                let mut source = source.to_owned();
                if !math {
                    source.insert_str(
                        0,
                        "$$\n# heading\none\ntwo\nthree\nfour\nfive\n```rust\nx\n```\n$$\n\n",
                    );
                }
                let chunks: Vec<&str> = source.split_inclusive('\n').collect();
                assert_stream_matches_full_render(&chunks, Some(80));
            }
        }
    }
}

/// A top-level Mermaid fence arriving a line at a time: while it is open
/// the rows appended a line at a time (`code_fence.rs`) are the whole
/// render's, links and spans included; its closer gives the diagram.
#[test]
fn an_open_mermaid_fence_appends_the_whole_renders_rows() {
    let chunks = [
        "Intro.\n\n",
        "```mermaid\n",
        "flowchart LR\n",
        "A[Request https://example.com] --> B[Reply]\n",
        "B --> C\n",
        "```\n",
    ];
    let (_, render) = assert_stream_matches_full_render(&chunks, Some(80));
    let rows = lines_to_plain_strings(&crate::terminal_hyperlinks::visible_lines(
        render.lines.clone(),
    ));
    assert!(rows.iter().any(|row| row.starts_with('┌')), "{rows:#?}");
    assert!(render.lines.iter().all(|line| line.hyperlinks.is_empty()));
}
