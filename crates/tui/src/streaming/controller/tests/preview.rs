// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use ratatui::text::Line;

use super::{finalized_cell, stream_controller};
use crate::history_cell::{AgentMessageCell, HistoryCell, StreamId};
use crate::markdown_render::ListSpacing;

fn plain(lines: &[Line<'static>]) -> Vec<String> {
    lines.iter().map(Line::to_string).collect()
}

fn render_source(source: &str, width: Option<usize>) -> Vec<Line<'static>> {
    crate::markdown::render_streaming_markdown_agent(
        source,
        width,
        Some("/repo"),
        ListSpacing::AfterMultiline,
    )
    .lines
}

#[test]
fn unfinished_link_destinations_do_not_expand_the_preview() {
    for destination in [
        "https://example.com/a/very/long/path/to/the/guide",
        "https://example.com/a_(nested_(path))",
        r"https://example.com/escaped\)parenthesis",
        "<https://example.com/unbalanced)parenthesis>",
        " <https://example.com/unbalanced)parenthesis>",
        "https://example.com/guide \"a title with ) parentheses\"",
        "https://example.com/guide (x < y)",
    ] {
        let mut controller = stream_controller(Some(24));
        controller.push("See [documentation]");
        let label = controller.tail_lines();
        // Split at every character, the opening delimiter and escapes too.
        for character in format!("({destination}").chars() {
            controller.push(&character.to_string());
            assert_eq!(controller.tail_lines(), label, "{destination}");
        }
        controller.push(") and continue.");
        assert_eq!(
            controller.tail_lines(),
            render_source(
                &format!("See [documentation]({destination}) and continue."),
                Some(24)
            ),
        );
    }
    // The preview's byte bound must not drop the opener and show the URL.
    let mut controller = stream_controller(Some(24));
    controller.push("See [documentation]");
    let label = controller.tail_lines();
    controller.push(&format!("(https://example.com/{}", "界".repeat(3000)));
    controller.push("more-path");
    assert_eq!(controller.tail_lines(), label);
}

#[test]
fn unfinished_links_flush_on_newline_or_completion() {
    let source = "See [documentation](https://example.com/a/very/long/path";
    for ending in ["", "\n"] {
        let mut controller = stream_controller(Some(24));
        controller.push(source);
        assert_eq!(plain(&controller.tail_lines()), vec!["See [documentation]"]);
        controller.set_width(Some(32));
        assert_eq!(plain(&controller.tail_lines()), vec!["See [documentation]"]);
        controller.push(ending);
        let (queued, _) = controller.on_commit_tick_batch(usize::MAX);
        let finalized = controller.finalize();
        let displayed = queued
            .as_deref()
            .into_iter()
            .chain(finalized_cell(&finalized))
            .flat_map(|cell| cell.display_lines(34))
            .collect::<Vec<_>>();
        let expected =
            AgentMessageCell::new(render_source(source, Some(32)), true, StreamId::next());
        assert_eq!(displayed, expected.display_lines(34));
    }
}

#[test]
fn link_preview_respects_code_and_escapes() {
    for source in [
        r"See \[label](https://example.com/unfinished",
        "See `[label](https://example.com/unfinished",
    ] {
        let mut controller = stream_controller(Some(80));
        controller.push(source);
        assert_eq!(
            controller.tail_lines(),
            render_source(source, Some(80)),
            "{source}"
        );
    }
}

#[test]
fn unterminated_prose_reflows_and_finishes_without_duplication() {
    for ending in ["", "\n"] {
        let mut controller = stream_controller(Some(32));
        let mut source = String::from("`numbers` ");
        controller.push(&source);
        for number in 100..200 {
            let delta = format!("{number} ");
            source.push_str(&delta);
            controller.push(&delta);
            assert_eq!(controller.queued_lines(), 0);
            assert_eq!(controller.tail_lines(), render_source(&source, Some(32)));
        }

        controller.push(" | pending structure");
        controller.set_width(Some(48));
        assert_eq!(controller.queued_lines(), 0);
        assert_eq!(controller.tail_lines(), render_source(&source, Some(48)));
        source.push_str(" | pending structure");
        controller.push(ending);
        let (queued, _) = controller.on_commit_tick_batch(usize::MAX);
        let finalized = controller.finalize();
        let displayed = queued
            .as_deref()
            .into_iter()
            .chain(finalized_cell(&finalized))
            .flat_map(|cell| cell.display_lines(50))
            .collect::<Vec<_>>();
        let expected =
            AgentMessageCell::new(render_source(&source, Some(48)), true, StreamId::next());
        assert_eq!(displayed, expected.display_lines(50));
    }
}

#[test]
fn partial_markdown_structures_remain_newline_gated() {
    for (committed, partial) in [
        ("", "| partial header"),
        ("", "```rust"),
        ("", "> ~~~rust"),
        ("```rust\n", "let partial = 1;"),
        ("- ```rust\n", "  let partial = 1;"),
        ("> - ```rust\n", ">   let partial = 1;"),
        ("    indented code\n", "    partial code"),
        ("```markdown\n", "partial markdown fence"),
        ("| A | B |\n| --- | --- |\n", "partial row"),
    ] {
        let mut controller = stream_controller(Some(40));
        controller.push(committed);
        let tail = controller.tail_lines();
        let queued = controller.queued_lines();
        controller.push(partial);
        assert_eq!(controller.tail_lines(), tail, "{committed:?} {partial:?}");
        assert_eq!(controller.queued_lines(), queued);
    }
}

#[test]
fn long_unicode_preview_keeps_recent_text_and_finalizes_full_source() {
    let mut controller = stream_controller(Some(40));
    controller.push(&"🦀 ".repeat(3000));
    controller.push("latest text");
    let preview = controller.tail_lines();
    assert_eq!(preview.first(), Some(&Line::from("…")));
    let text = preview
        .iter()
        .map(Line::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.trim_end().ends_with("latest text"), "{text}");
    assert_eq!(controller.queued_lines(), 0);
    let finalized = controller.finalize();
    let rows = finalized_cell(&finalized)
        .map(|cell| cell.display_lines(42).len())
        .unwrap_or_default();
    assert_eq!(
        rows,
        render_source(&format!("{}latest text", "🦀 ".repeat(3000)), Some(40)).len()
    );
}

#[test]
fn prose_preview_resumes_after_fenced_code() {
    let mut controller = stream_controller(Some(40));
    controller.push("```rust\nlet x = 1;\n```\n");
    controller.on_commit_tick_batch(usize::MAX);
    controller.push("prose after code");
    assert_eq!(
        controller.tail_lines(),
        vec![Line::from("prose after code")]
    );
}

#[test]
fn prose_after_a_table_is_previewed_until_another_table_starts() {
    let mut controller = stream_controller(Some(40));
    controller.push("| A | B |\n| --- | --- |\n| a | b |\n\n");
    controller.push("prose after the table");
    let tail = controller.tail_lines();
    assert_eq!(tail.last(), Some(&Line::from("prose after the table")));
    assert_eq!(controller.queued_lines(), 0);

    controller.push("\n\n| C | D |\n| --- | --- |\n");
    let tail = controller.tail_lines();
    controller.push("partial row");
    assert_eq!(controller.tail_lines(), tail);
}
