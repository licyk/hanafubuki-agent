// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Mermaid while it streams: an open fence is code held as the tail, its
//! closer turns it into the diagram, still the tail's until another block
//! starts, and a resize can turn the diagram back into its code. Whatever
//! the chunks, the rows committed equal the full render, with diagrams on
//! or off. Codex's streaming tests (their rich halves: this interface has
//! no raw stream, nor copy targets) and samples of every family, nested,
//! unwrapped from a ```` ```md ```` fence, falling back, and cut off.

use std::time::{Duration, Instant};

use hanafubuki_agent::AgentTuiRendering;
use insta::assert_snapshot;

use super::equality::{arbitrary_chunks, by_char, stream};
use super::{full_render, lines_to_plain_strings, stream_controller};
use crate::markdown::render_streaming_markdown_agent;
use crate::markdown_render::ListSpacing;
use crate::streaming::render::StreamingRender;
use crate::terminal_hyperlinks::visible_lines;

const CWD: &str = "/repo";

fn new_render() -> StreamingRender {
    StreamingRender::new(render_streaming_markdown_agent, ListSpacing::AfterMultiline)
}

fn text(render: &StreamingRender) -> String {
    lines_to_plain_strings(&visible_lines(render.lines.clone())).join("\n")
}

#[test]
fn mermaid_stream_closes_resizes_and_preserves_raw_source() {
    let mut render = new_render();
    let mut source = String::new();
    let mut stages = Vec::new();
    for chunk in [
        "Before.\n\n```mermaid\n",
        "flowchart LR\nA[\"Codex reply<br/>completed\"] --> B[\"Rendered in Ghostty\"]\n",
        "```\n",
        "\nAfter.\n",
    ] {
        source.push_str(chunk);
        render.append(&source, chunk, Some(80), CWD);
        assert_eq!(
            render.lines,
            render.render_source(&source, Some(80), CWD).lines
        );
        stages.push(text(&render));
    }
    for width in [8, 80] {
        render.recompute(&source, Some(width), CWD);
        assert_eq!(
            render.lines,
            render.render_source(&source, Some(width), CWD).lines
        );
        stages.push(text(&render));
    }
    assert_snapshot!(stages.join("\n\n---\n\n"));
}

#[test]
fn mermaid_holdback_tracks_nested_blocks_after_normalized_markdown() {
    let prefix = "```md\n| Name | Value |\n| --- | --- |\n| A | B |\n```\n\n";
    for block in [
        "> ```mermaid\n> flowchart LR\n> A --> B\n> ```\n",
        "- Diagram:\n\n  ```mermaid\n  flowchart LR\n  A --> B\n  ```\n",
    ] {
        let mut render = new_render();
        let mut source = prefix.to_owned();
        render.append(&source, prefix, Some(80), CWD);
        for chunk in block.split_inclusive('\n') {
            source.push_str(chunk);
            render.append(&source, chunk, Some(80), CWD);
        }
        assert_eq!(render.mutable_fence_start, Some(prefix.len()));
        render.recompute(&source, Some(80), CWD);
        assert_eq!(render.mutable_fence_start, Some(prefix.len()));
        let continuation = "\nAfter.\n\n";
        source.push_str(continuation);
        render.append(&source, continuation, Some(80), CWD);
        assert_eq!(render.mutable_fence_start, None);
        render.recompute(&source, Some(80), CWD);
        assert_eq!(render.mutable_fence_start, None);
    }
}

fn tail_text(lines: &[crate::terminal_hyperlinks::HyperlinkLine]) -> Vec<String> {
    lines_to_plain_strings(&visible_lines(lines.to_vec()))
}

#[test]
fn mermaid_controller_keeps_diagram_mutable_and_returns_original_source() {
    let mut controller = stream_controller(Some(80));
    let source = "```mermaid\nflowchart LR\nA[Request<br />completed] --> B[Reply]\n```\n";
    for chunk in source.split_inclusive('\n') {
        controller.push(chunk);
        assert!(
            controller.on_commit_tick().0.is_none(),
            "Mermaid escaped into scrollback"
        );
    }
    let diagram = controller.tail_lines();
    assert!(tail_text(&diagram).iter().any(|line| line.contains('┌')));
    controller.set_width(Some(8));
    assert!(tail_text(&controller.tail_lines())
        .iter()
        .any(|line| line.contains("A[Request<br />completed]")));
    controller.set_width(Some(80));
    assert_eq!(controller.tail_lines(), diagram);
    let mut full_source = source.to_owned();
    for i in 0..256 {
        let continuation = format!("\nStep {i}\n");
        full_source.push_str(&continuation);
        controller.push(&continuation);
        assert!(!controller.has_live_tail());
        assert!(controller.tail_lines().is_empty());
        assert!(controller.on_commit_tick_batch(usize::MAX).0.is_some());
    }
    controller.set_width(Some(8));
    controller.set_width(Some(80));
    full_source.push('\n');
    controller.push("\n");
    controller.on_commit_tick_batch(usize::MAX);
    for chunk in source.split_inclusive('\n') {
        full_source.push_str(chunk);
        controller.push(chunk);
        assert!(controller.on_commit_tick().0.is_none());
    }
    assert!(controller.has_live_tail());
    let second_diagram = controller.tail_lines();
    assert!(second_diagram.len() <= diagram.len() + 1);
    controller.set_width(Some(8));
    assert!(tail_text(&controller.tail_lines())
        .iter()
        .any(|line| line.contains("A[Request<br />completed]")));
    controller.set_width(Some(80));
    assert_eq!(controller.tail_lines(), second_diagram);
    assert_eq!(controller.committed_source(), full_source);
}

/// Diagrams of every family, nested in a list and a quote, unwrapped from
/// a ```` ```md ```` fence, wider than the narrow widths, unsupported,
/// with CJK and emoji labels, after and before prose, with CRLF line
/// endings, and cut off before their closer.
const SAMPLES: &[&str] = &[
    "Plan:\n\n```mermaid\nflowchart TD\n    A([Start]) --> B{Ready?}\n    B -->|yes| C[Ship]\n    B -->|no| A\n```\n\nAfter the diagram.\n",
    "```mermaid\nsequenceDiagram\n    participant U as User\n    U->>API: request\n    API-->>U: response\n```\n",
    "```mermaid\nstateDiagram-v2\n    [*] --> Draft\n    Draft --> Done: submit\n    Done --> [*]\n```\nText right after.\n",
    "```mermaid\nclassDiagram\n    class Order {\n        +submit()\n    }\n    Order \"1\" *-- \"many\" Item : contains\n```\n",
    "```mermaid\nerDiagram\n    CUSTOMER ||--o{ ORDER : places\n```\n\n```mermaid\ngraph LR; A --> B\n```\n",
    "- Diagram:\n\n  ```mermaid\n  flowchart LR\n  A[请求] --> B[Ship 🚀]\n  ```\n\n- Next item\n",
    "> ```mermaid\n> flowchart TD\n> A --> B\n> ```\n>\n> quoted after\n",
    "````md\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```mermaid\nflowchart LR\nA --> B\n```\n````\n\nAfter the fence.\n",
    "```mermaid\npie\n\"Cats\": 2\n```\n\nAfter the notice.\n",
    "```mermaid\r\nflowchart LR\r\nA --> B\r\n```\r\n\r\nAfter.\r\n",
    "Intro.\n\n```mermaid\nflowchart LR\nA --> B\n",
    "Intro.\n\n~~~mermaid\nsequenceDiagram\nA->>B: hi",
];

fn assert_samples_stream_to_their_full_render() {
    for source in SAMPLES {
        for width in [None, Some(80), Some(40), Some(20)] {
            let expected = full_render(source, width);
            let splits: Vec<(&str, Vec<&str>)> = vec![
                ("lines", source.split_inclusive('\n').collect()),
                ("chars", by_char(source)),
                ("chunks 3", arbitrary_chunks(source, 3)),
                ("chunks 8", arbitrary_chunks(source, 8)),
                ("whole", vec![source]),
            ];
            for (split, chunks) in splits {
                let (streamed, diverged) = stream(&chunks, width, ListSpacing::AfterMultiline);
                assert_eq!(streamed, expected, "{split} at {width:?}:\n{source}");
                assert!(!diverged, "{split} at {width:?}:\n{source}");
            }
        }
    }
}

#[test]
fn mermaid_streams_to_its_full_render_however_it_is_split() {
    assert_samples_stream_to_their_full_render();
}

#[test]
fn mermaid_off_streams_to_its_full_render_too() {
    crate::markdown_render::preferences::set(AgentTuiRendering {
        mermaid: false,
        ..Default::default()
    });
    assert_samples_stream_to_their_full_render();
}

/// While the fence is open its code shows in the tail and nothing of it is
/// committed, line after line; once it closes the tail is the diagram.
#[test]
fn an_open_fence_is_code_in_the_tail_until_its_closer() {
    let body = ["flowchart LR\n", "A[Request] --> B[Reply]\n", "B --> C\n"];
    let mut stream = crate::streaming::controller::TestStream::new(Some(80), CWD);
    stream.push("Before.\n\n```mermaid\n");
    let committed = stream.commit();
    assert_eq!(lines_to_plain_strings(&committed), ["Before."]);
    for line in body {
        stream.push(line);
        assert!(stream.commit().is_empty(), "a row of the open fence");
        assert!(lines_to_plain_strings(&stream.tail()).contains(&line.trim_end().to_string()));
    }
    stream.push("```\n");
    assert!(stream.commit().is_empty());
    // The blank row before the fence, then the diagram.
    let tail = lines_to_plain_strings(&stream.tail());
    assert_eq!(tail[0], "");
    assert!(tail[1].starts_with('┌'), "{tail:#?}");
    assert!(!tail.iter().any(|row| row.contains("-->")), "{tail:#?}");
    stream.push("\nAfter.\n");
    let committed = lines_to_plain_strings(&stream.commit());
    assert_eq!(committed[..tail.len()], tail[..]);
    assert!(stream.tail().is_empty());
}

/// A fence of 10,000 lines streamed a line at a time stays code, held,
/// in a bounded time: its rows are appended a line at a time, never the
/// whole fence rendered again per line. A diagram of that size is past
/// the 16 KiB limit and falls back at its closer.
#[test]
fn a_huge_open_fence_streams_in_bounded_time() {
    let mut stream = crate::streaming::controller::TestStream::new(Some(80), CWD);
    let started = Instant::now();
    stream.push("```mermaid\nflowchart TD\n");
    for i in 0..10_000 {
        stream.push(&format!("N{i} --> N{}\n", i + 1));
        assert!(stream.commit().is_empty());
    }
    stream.push("```\n");
    let (rest, diverged) = stream.finalize_checked();
    let elapsed = started.elapsed();
    assert!(!diverged);
    let rest = lines_to_plain_strings(&rest);
    assert!(
        rest[0].starts_with("This Mermaid diagram exceeds"),
        "{:?}",
        &rest[..2]
    );
    assert_eq!(rest.len(), 10_002);
    assert!(elapsed < Duration::from_secs(60), "{elapsed:?}");
}
