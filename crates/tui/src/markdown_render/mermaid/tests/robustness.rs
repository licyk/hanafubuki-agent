//! A model writes the fence, so any text may reach the renderer: valid
//! diagrams cut at every character, generated punctuation, deep nesting
//! and huge inputs, through the writer and the renderer alone, at every
//! kind of width. None panics, none takes long, and each gives either a
//! whole diagram within its width or the notice and the code.

use std::time::{Duration, Instant};

use super::markdown_text;
use crate::mermaid::{render_spans, RenderError};
use crate::wrapping::display_width;

const VALID: [&str; 6] = [
    "flowchart TD\n    A([\"请求<br>Ready\"]) -- go --> B[Work] & C{Done?}\n    B -. no .-> C\n    C <--> A\n    B --- A\n",
    "graph LR; A -->|准备| B; B{Réponse?}; B -->|retry| A; B --> C[Ship 🚀]",
    "sequenceDiagram\n    actor U as Buyer\n    U->>A: hi\n    opt Items\n        loop Retry\n            A->>A: check\n            alt ok\n                Note over U,A: done\n                A-->>U: ok\n            else no\n                A--xU: no\n            end\n        end\n    end\n",
    "stateDiagram-v2\n    state \"Payment pending\" as P\n    [*] --> Draft\n    Draft --> P: submit\n    P --> [*]\n    P: Retry\n",
    "classDiagram\n    class Order {\n        +int[] ids\n        +submit()\n    }\n    Order \"1\" *-- \"1..*\" Item : contains\n    Item ..|> Thing\n",
    "erDiagram\n    A ||--o{ B : has\n    B {\n        int id PK, FK \"key\"\n    }\n",
];

/// What the writer shows: a diagram's rows within the width and without
/// control characters; otherwise the code, as any code block shows it.
fn assert_sane(fenced: &str, width: usize) {
    let text = markdown_text(fenced, width);
    let diagram = text.starts_with(['┌', '╭', ' ']) && !text.contains("Mermaid diagram");
    if diagram {
        assert!(
            text.lines().all(|row| display_width(row) <= width),
            "{fenced:?} at {width}:\n{text}"
        );
        assert!(
            !text.chars().any(|ch| ch.is_control() && ch != '\n'),
            "{fenced:?} at {width}"
        );
    }
}

/// Every width a renderer may be asked, the extremes included, straight
/// to the renderer: a drawing never wider than asked.
fn assert_renders_or_refuses(source: &str) {
    for width in [0, 1, 7, 40, 120, 1_000, usize::MAX] {
        if let Ok(rows) = render_spans(source, width) {
            for row in rows {
                let text = row
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>();
                assert!(display_width(&text) <= width, "{source:?} at {width}");
                assert!(!text.chars().any(char::is_control), "{source:?}");
            }
        }
    }
}

/// A fixed sequence of numbers, the same on every run and platform.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as usize
    }
}

#[test]
fn every_truncation_of_a_valid_diagram_draws_whole_or_falls_back() {
    let started = Instant::now();
    for source in VALID {
        for end in source
            .char_indices()
            .map(|(index, _)| index)
            .chain([source.len()])
        {
            let cut = &source[..end];
            assert_renders_or_refuses(cut);
            assert_sane(&format!("```mermaid\n{cut}\n```\n"), 60);
            // Cut inside the fence too: no closer, the code as it is.
            assert_sane(&format!("```mermaid\n{cut}"), 60);
        }
    }
    assert!(started.elapsed() < Duration::from_secs(60));
}

#[test]
fn generated_punctuation_never_panics() {
    const ALPHABET: &[&str] = &[
        "A",
        "B",
        "x",
        "o",
        " ",
        " ",
        "\n",
        ";",
        "&",
        "|",
        ":",
        "\"",
        "[",
        "]",
        "{",
        "}",
        "(",
        ")",
        "<",
        ">",
        "-",
        "-",
        ".",
        "=",
        "*",
        "%%",
        "#",
        "<br>",
        "-->",
        "->>",
        "--x",
        "||",
        "o{",
        "end",
        "loop a",
        "alt b",
        "else c",
        "Note over A,B: n",
        "[*]",
        "class ",
        "state ",
        "direction LR",
        "界",
        "🚀",
        "\u{301}",
        "\t",
        "\r",
    ];
    const HEADERS: &[&str] = &[
        "flowchart TD\n",
        "graph LR;",
        "sequenceDiagram\n",
        "stateDiagram-v2\n",
        "classDiagram\n",
        "erDiagram\n",
        "",
    ];
    let started = Instant::now();
    let mut random = Lcg(0x006d_6572_6d61_6964);
    let mut drawn = 0;
    for case in 0..4_000 {
        let mut source = HEADERS[case % HEADERS.len()].to_string();
        for _ in 0..random.next() % 80 {
            source.push_str(ALPHABET[random.next() % ALPHABET.len()]);
        }
        assert_renders_or_refuses(&source);
        drawn += usize::from(render_spans(&source, 200).is_ok());
        if case % 8 == 0 {
            assert_sane(&format!("```mermaid\n{source}\n```\n"), 40);
        }
    }
    // Some of them are diagrams: the sweep reaches the drawing too.
    assert!(drawn > 0);
    assert!(started.elapsed() < Duration::from_secs(60));
}

#[test]
fn deep_nesting_is_refused_at_its_limit() {
    let loops = format!(
        "sequenceDiagram\n{}A->>B: hi\n{}",
        "loop again\n".repeat(1_000),
        "end\n".repeat(1_000)
    );
    assert_eq!(render_spans(&loops, 200), Err(RenderError::Limit));
    for source in [
        format!("flowchart TD\nA{}B{}", "[".repeat(5_000), "]".repeat(5_000)),
        format!("flowchart TD\nA{}", "([".repeat(5_000)),
        format!("classDiagram\n{}", "class A {\n".repeat(1_000)),
        format!("erDiagram\n{}", "A {\n".repeat(1_000)),
        format!("flowchart TD\n{}", "A & ".repeat(3_000) + "B --> C"),
    ] {
        assert!(render_spans(&source, 200).is_err(), "{}", &source[..40]);
    }
    let quoted = format!(
        "{0}```mermaid\n{0}flowchart LR\n{0}A --> B\n{0}```\n",
        "> ".repeat(100)
    );
    assert_sane(&quoted, 80);
    assert_sane(&quoted, 400);
}

#[test]
fn huge_inputs_are_refused_at_once() {
    let started = Instant::now();
    let lines = (0..10_000)
        .map(|n| format!("N{n} --> N{}\n", n + 1))
        .collect::<String>();
    let source = format!("flowchart TD\n{lines}");
    assert_eq!(render_spans(&source, usize::MAX), Err(RenderError::Limit));
    let text = markdown_text(&format!("```mermaid\n{source}```\n"), 80);
    assert!(text.starts_with("This Mermaid diagram exceeds"));
    assert_eq!(text.lines().count(), 10_002);
    // Just under the source limit: refused by the graph's own limits.
    let edges = "A-->B;".repeat(16 * 1024 / 6 - 4);
    let source = format!("graph TD;{edges}");
    assert!(source.len() <= 16 * 1024);
    assert_eq!(render_spans(&source, usize::MAX), Err(RenderError::Limit));
    let long_line = format!("flowchart TD\nA[{}]", "x".repeat(16_000));
    assert_eq!(
        render_spans(&long_line, usize::MAX),
        Err(RenderError::Limit)
    );
    assert!(started.elapsed() < Duration::from_secs(60));
}
