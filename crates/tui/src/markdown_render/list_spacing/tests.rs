// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Uniform list spacing, Codex's samples: nested lists, quoted lists,
//! paragraphs, a code block that starts an item, an item wider than the
//! width. `Compact` (streaming) is not here. The snapshot is Codex's
//! `Uniform` stages but for the long URL, which this crate's wrapping
//! splits until M4 ports URL-aware wrapping.

use super::ListSpacing;
use crate::markdown::render_markdown_agent_with_list_spacing;

fn render(source: &str, width: usize, spacing: ListSpacing) -> Vec<String> {
    render_markdown_agent_with_list_spacing(source, Some(width), None, spacing)
        .iter()
        .map(|line| line.to_string().trim_end().to_owned())
        .collect()
}

#[test]
fn list_spacing_is_uniform_per_list_and_preserves_item_paragraphs() {
    let sources = [
        "1. One\n2. This item has enough words to wrap\n3. Three\n4. Four",
        "- Parent\n  - Short\n  - Small\n- Other\n\n> - Quote\n> - This quoted item has enough words to wrap\n> - End",
        "- First paragraph\n\n  Second paragraph\n\n- Next\n\n- Last",
        "- One\n- ```text\n  code\n  ```\n- Three\n\n  ```text\n  after paragraph\n  ```",
        "- https://example.com/a-very-long-path-with-many-segments\n- Short\n- End",
    ];
    let stages = sources
        .iter()
        .map(|source| render(source, 24, ListSpacing::Uniform).join("\n"))
        .collect::<Vec<_>>();
    insta::assert_snapshot!(stages.join("\n\n---\n\n"));
}

#[test]
fn list_spacing_keeps_lists_without_a_multiline_item_tight() {
    let source = "- [One](https://example.com/one)\n- [Two](https://example.com/two)\n- Three";
    assert_eq!(
        render(source, 100, ListSpacing::AfterMultiline),
        render(source, 100, ListSpacing::Uniform)
    );
}
