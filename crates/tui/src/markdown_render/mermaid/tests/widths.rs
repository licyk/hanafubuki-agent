//! Mermaid fences as the writer shows them: every family at 120, 80 and
//! 40 columns (a diagram, or the notice and the code where it does not
//! fit), each fallback, wide labels, a diagram in a list and a quote, one
//! unwrapped from a ```` ```md ```` fence, never linked, the same at a
//! width again after another, and the setting off.

use hanafubuki_agent::AgentTuiRendering;
use insta::assert_snapshot;

use super::markdown_text;
use crate::markdown::append_markdown_agent;
use crate::markdown_render::{
    preferences, render_markdown_lines, render_markdown_text_with_width, ListSpacing,
};
use crate::wrapping::display_width;

const FAMILIES: [(&str, &str); 6] = [
    (
        "flowchart",
        "flowchart TD\n    A([Start]) --> B{Ready?}\n    B -->|yes| C[Ship it]\n    B -->|no| D[Fix]\n    D --> B\n",
    ),
    ("flowchart LR", "graph LR\n    A[Request] --> B[Reply] --> C[Done]\n"),
    (
        "sequence",
        "sequenceDiagram\n    participant U as User\n    participant A as API\n    U->>A: request\n    alt ok\n        A-->>U: response\n    else failed\n        A--xU: error\n    end\n    Note over U,A: done\n",
    ),
    (
        "state",
        "stateDiagram-v2\n    [*] --> Draft\n    Draft --> Review: submit\n    Review --> Draft: revise\n    Review --> [*]: merge\n",
    ),
    (
        "class",
        "classDiagram\n    class Order {\n        +int id\n        +submit()\n    }\n    Order \"1\" *-- \"many\" Item : contains\n    Payment <|-- Card\n",
    ),
    (
        "er",
        "erDiagram\n    CUSTOMER ||--o{ ORDER : places\n    ORDER {\n        int id PK\n        string status\n    }\n",
    ),
];

fn fence(source: &str) -> String {
    format!("```mermaid\n{source}```\n")
}

fn assert_within(text: &str, width: usize) {
    for row in text.lines() {
        assert!(display_width(row) <= width, "{row:?} wider than {width}");
    }
}

#[test]
fn every_family_at_120_80_and_40_columns() {
    let mut cases = Vec::new();
    for (name, source) in FAMILIES {
        for width in [120, 80, 40] {
            let text = markdown_text(&fence(source), width);
            assert_within(&text, width);
            cases.push(format!("{name} at {width}\n{text}"));
        }
    }
    assert_snapshot!(cases.join("\n\n---\n\n"));
}

/// What cannot be drawn shows a dim notice saying why, then the code as an
/// ordinary block shows it; an empty fence shows nothing, as before.
#[test]
fn each_fallback_shows_its_notice_and_the_code() {
    let unsupported = "This Mermaid diagram uses features the terminal renderer doesn't support.";
    let limit = "This Mermaid diagram exceeds the terminal renderer's size limits.";
    let too_wide = "This Mermaid diagram doesn't fit the current terminal width.";
    let seventeen = (0..17)
        .map(|n| format!("N{n} --> N{}\n", n + 1))
        .collect::<String>();
    let wide = (0..8)
        .map(|n| format!("N{n}[A label of some length {n}] --> "))
        .collect::<String>();
    for (body, notice) in [
        ("pie\n\"Cats\": 2\n", unsupported),
        ("gantt\ntitle A plan\n", unsupported),
        ("journey\ntitle A day\n", unsupported),
        ("gitGraph\ncommit\n", unsupported),
        ("mindmap\nroot\n", unsupported),
        ("flowchart TD\nA --> B -->\n", unsupported),
        ("flowchart TD\nsubgraph X\nA --> B\nend\n", unsupported),
        ("%% only a comment\n%% and another\n", unsupported),
        ("flowchart TD\n%%{init: {}}%%\nA --> B\n", unsupported),
        (&format!("flowchart TD\n{seventeen}"), limit),
        (&format!("flowchart LR\n{wide}Z\n"), too_wide),
    ] {
        let source = fence(body);
        let rendered = render_markdown_text_with_width(&source, Some(120));
        let text = rendered.to_string();
        assert!(text.starts_with(notice), "{body:?}:\n{text}");
        assert!(rendered.lines[0]
            .spans
            .iter()
            .filter(|span| !span.content.is_empty())
            .all(|span| span
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::DIM)));
        assert!(
            text.ends_with(&markdown_text(
                &source.replacen("mermaid", "unknown", 1),
                120
            )),
            "{body:?}:\n{text}"
        );
    }
    assert_eq!(markdown_text("```mermaid\n```\n", 80), "");
    assert_eq!(
        markdown_text("Before.\n\n```mermaid\n```\n\nAfter.\n", 80),
        markdown_text("Before.\n\n```unknown\n```\n\nAfter.\n", 80)
    );
}

#[test]
fn crlf_line_endings_draw_the_same_diagram() {
    for (_, source) in FAMILIES {
        let crlf = fence(source).replace('\n', "\r\n");
        assert_eq!(
            markdown_text(&crlf, 120),
            markdown_text(&fence(source), 120)
        );
    }
}

#[test]
fn cjk_and_emoji_labels_keep_the_boxes_aligned() {
    let source = fence(
        "flowchart TD\n    A[请求处理] --> B{完成了吗？}\n    B -->|是| C[发布 🚀]\n    B -->|否| A\n",
    );
    let text = markdown_text(&source, 80);
    assert!(text.starts_with('┌'), "{text}");
    for label in ["请求处理", "◇ 完成了吗？", "发布 🚀", "是", "否"] {
        assert!(text.contains(label), "{label}:\n{text}");
    }
    // Every box's right border is in one column: the label's wide
    // characters are counted twice.
    let borders = text
        .lines()
        .filter(|row| row.starts_with('│'))
        .map(|row| {
            let end = row
                .char_indices()
                .skip(1)
                .find(|(_, ch)| matches!(ch, '│' | '├'));
            display_width(&row[..end.expect("a right border").0])
        })
        .collect::<Vec<_>>();
    assert!(borders.windows(2).all(|pair| pair[0] == pair[1]), "{text}");
    let sequence = fence("sequenceDiagram\n    participant 客户\n    客户->>服务: 你好\n");
    // Identifiers are ASCII: the fence is shown as code.
    assert!(markdown_text(&sequence, 80).starts_with("This Mermaid diagram uses"));
    let sequence = fence("sequenceDiagram\n    participant C as 客户 🙂\n    C->>S: 你好\n");
    let text = markdown_text(&sequence, 80);
    assert!(text.contains("│ 客户 🙂 │"), "{text}");
    assert_within(&text, 80);
}

/// In a list item or a quote the diagram fits the width less the prefix,
/// and falls back where that is too narrow.
#[test]
fn a_diagram_in_a_list_item_and_a_quote_fits_under_its_prefix() {
    let body = "flowchart LR\nA[Request] --> B[Reply]\n";
    let listed = format!("- Diagram:\n\n  ```mermaid\n{}  ```\n", indent(body, "  "));
    let quoted = format!("> ```mermaid\n{}> ```\n", indent(body, "> "));
    let nested = format!(
        "> 1. Step\n>\n>    ```mermaid\n{}>    ```\n",
        indent(body, ">    ")
    );
    let natural = crate::mermaid::render(body, 200).unwrap();
    let natural_width = natural.lines().map(display_width).max().unwrap();
    for (source, prefix) in [(listed, 2), (quoted, 2), (nested, 5)] {
        let fits = markdown_text(&source, natural_width + prefix);
        assert!(fits.contains("┌─────────┐"), "{fits}");
        assert_within(&fits, natural_width + prefix);
        let narrow = markdown_text(&source, natural_width + prefix - 1);
        assert!(narrow.contains("doesn't fit"), "{narrow}");
    }
}

fn indent(body: &str, prefix: &str) -> String {
    body.lines()
        .map(|line| format!("{prefix}{line}\n"))
        .collect()
}

/// An answer's ```` ````md ```` fence holding a table and a diagram is
/// unwrapped: the table laid out, the diagram drawn.
#[test]
fn a_diagram_unwrapped_from_a_markdown_fence() {
    let source =
        "````md\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```mermaid\nflowchart LR\nA --> B\n```\n````\n";
    let mut lines = Vec::new();
    append_markdown_agent(source, Some(80), None, &mut lines);
    let text = lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("```"), "{text}");
    assert!(text.contains("┌───┐  ┌───┐"), "{text}");
    assert!(text.contains('━'), "the table:\n{text}");
}

/// A diagram's labels are text: no web address in one is linked or takes
/// the link style.
#[test]
fn nothing_in_a_diagram_is_linked() {
    let source = fence(
        "sequenceDiagram\n    A->>B: see https://example.com/a\n    Note over A,B: www.example.com and https://example.com/b\n",
    );
    let lines = render_markdown_lines(&source, Some(120), None, ListSpacing::AfterMultiline);
    assert!(lines
        .iter()
        .any(|line| line.line.to_string().contains("https://example.com/a")));
    assert!(lines.iter().all(|line| line.hyperlinks.is_empty()));
    let flowchart = fence("flowchart LR\n    A[https://example.com] -->|http://x.io| B\n");
    let lines = render_markdown_lines(&flowchart, Some(120), None, ListSpacing::AfterMultiline);
    assert!(lines[0].line.to_string().starts_with('┌'));
    assert!(lines.iter().all(|line| line.hyperlinks.is_empty()));
    // The same address in prose is linked.
    let prose = render_markdown_lines(
        "https://example.com/a",
        Some(120),
        None,
        ListSpacing::AfterMultiline,
    );
    assert!(!prose[0].hyperlinks.is_empty());
}

/// Laid out for another width, a diagram that no longer fits is its code,
/// and the diagram again once it fits, both ways.
#[test]
fn a_resize_switches_between_diagram_and_code_both_ways() {
    let source = format!("Before.\n\n{}\nAfter.\n", fence(FAMILIES[1].1));
    let wide = markdown_text(&source, 100);
    let narrow = markdown_text(&source, 24);
    assert!(wide.contains('┌') && !wide.contains("-->"), "{wide}");
    assert!(
        narrow.contains("doesn't fit") && narrow.contains("-->"),
        "{narrow}"
    );
    assert_eq!(markdown_text(&source, 24), narrow);
    assert_eq!(markdown_text(&source, 100), wide);
}

/// With the setting off the fence is the code block it was before
/// diagrams, styles included: the same rows as a fence of no language
/// the highlighter knows.
#[test]
fn turned_off_a_fence_is_the_code_block_it_was() {
    preferences::set(AgentTuiRendering {
        mermaid: false,
        ..Default::default()
    });
    for (_, source) in FAMILIES {
        let mermaid = fence(source);
        let rendered = render_markdown_text_with_width(&mermaid, Some(80));
        assert_eq!(
            rendered,
            render_markdown_text_with_width(&mermaid.replacen("mermaid", "unknown", 1), Some(80))
        );
        let code = crate::render::highlight::highlight_code_to_lines(source, "mermaid");
        assert_eq!(
            rendered
                .lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            code.iter().map(ToString::to_string).collect::<Vec<_>>()
        );
    }
}
