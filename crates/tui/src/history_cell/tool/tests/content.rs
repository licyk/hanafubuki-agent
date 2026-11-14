// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A tool's text and items (Codex's `mcp_tests.rs`, where it applies): a
//! JSON text compacted collapsed and indented expanded, media and resources
//! as one-line summaries, `/raw` as the text came, at any width, whatever a
//! server sends.

use hanafubuki_agent::{AgentOutputBlock, AgentToolDisplay, ToolOutput, ToolOutputContent};
use serde_json::json;

use super::{both, render, shown, text};
use crate::history_cell::HistoryCell;
use crate::wrapping::line_width;

/// What the call shows, from its output as the fallback reads it (an MCP
/// tool has no presenter).
fn mcp(content: Vec<ToolOutputContent>) -> super::ToolCallCell {
    let output = ToolOutput {
        content,
        structured_content: None,
        is_error: false,
    };
    shown(
        "mcp__search__lookup",
        Some("rust"),
        AgentToolDisplay::fallback(&output),
    )
}

fn text_item(text: &str) -> ToolOutputContent {
    ToolOutputContent::Text {
        text: text.to_string(),
    }
}

fn raw(cell: &super::ToolCallCell) -> String {
    text(&cell.raw_lines())
}

/// Codex's `mcp_preview_shares_one_limit_across_blocks_and_preserves_transcript`:
/// one preview across the items, all of them expanded and raw.
#[test]
fn the_preview_shares_one_limit_across_items_and_the_transcript_keeps_all() {
    let cell = mcp(vec![
        text_item("first\nsecond"),
        text_item("third\nfourth\nfifth"),
    ]);
    insta::assert_snapshot!(both(&cell, 80), @r"
    • Called mcp__search__lookup: rust
      └ first
        second
        third
    ─── expanded (Some(OutputLines(2))) ───
    • Called mcp__search__lookup: rust
      └ first
        second
        third
        fourth
        fifth
    ");
    assert_eq!(
        raw(&cell),
        "• Called mcp__search__lookup: rust\n  └ first\n    second\n    third\n    fourth\n    fifth"
    );
}

/// Codex's `projected_content_preserves_full_rendering`: each item as it
/// shows. Media are the plan's summaries (`image/png · 34 KB`) where Codex
/// says `Returned image` and `<audio content>`; a text resource keeps its
/// text after its address.
#[test]
fn each_item_shows_as_text_or_a_one_line_summary() {
    let cell = mcp(vec![
        text_item(r#"{"result": [1, 2, 3], "text": "long output 🦀"}"#),
        ToolOutputContent::Image {
            mime_type: "image/png".to_string(),
            data: "iVBORw0KGgo=".to_string(),
            detail: None,
        },
        ToolOutputContent::Audio {
            mime_type: "audio/wav".to_string(),
            data: "A".repeat(4 * 12_000),
        },
        ToolOutputContent::EmbeddedResource {
            uri: "file:///text.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            text: Some("resource body".to_string()),
            data: None,
        },
        ToolOutputContent::EmbeddedResource {
            uri: "file:///blob.bin".to_string(),
            mime_type: Some("application/octet-stream".to_string()),
            text: None,
            data: Some("AAAA".to_string()),
        },
        ToolOutputContent::ResourceLink {
            name: "linked".to_string(),
            uri: "file:///linked.txt".to_string(),
            mime_type: None,
        },
        // What the MCP client makes of a block it cannot read.
        text_item("[unsupported MCP content: future_block]"),
    ]);
    insta::assert_snapshot!(both(&cell, 80), @r#"
    • Called mcp__search__lookup: rust
      └ {"result": [1, 2, 3], "text": "long output 🦀"}
        image/png · 8 B
        audio/wav · 35.2 KiB
    ─── expanded (Some(Generic)) ───
    • Called mcp__search__lookup: rust
      └ {
          "result": [
            1,
            2,
            3
          ],
          "text": "long output 🦀"
        }
        image/png · 8 B
        audio/wav · 35.2 KiB
        embedded resource: file:///text.txt
        resource body
        embedded resource: file:///blob.bin · application/octet-stream · 3 B
        link: file:///linked.txt
        [unsupported MCP content: future_block]
    "#);
}

#[test]
fn a_json_output_is_compact_collapsed_indented_expanded_and_as_it_came_raw() {
    let json = r#"{"name":"ratatui","versions":["0.29","0.30"],"meta":{"stars":10000,"empty":{}}}"#;
    let cell = mcp(vec![text_item(json)]);
    insta::assert_snapshot!(both(&cell, 60), @r#"
    • Called mcp__search__lookup: rust
      └ {"name": "ratatui", "versions": ["0.29", "0.30"], "meta…
    ─── expanded (Some(Generic)) ───
    • Called mcp__search__lookup: rust
      └ {
          "name": "ratatui",
          "versions": [
            "0.29",
            "0.30"
          ],
          "meta": {
            "stars": 10000,
            "empty": {}
          }
        }
    "#);
    assert_eq!(
        raw(&cell),
        format!("• Called mcp__search__lookup: rust\n  └ {json}")
    );
    // A text display (a plugin's) is laid out the same way.
    let plugin = shown(
        "lookup",
        None,
        AgentToolDisplay::Text {
            text: r#"[1,{"a":null}]"#.to_string(),
        },
    );
    assert_eq!(
        render(&plugin, 60),
        "• Called lookup\n  └ [1, {\"a\": null}]"
    );
}

/// What only looks like JSON, and JSON too large or too deep to lay out,
/// shows as it came.
#[test]
fn text_that_is_not_json_to_lay_out_shows_as_it_came() {
    let deep = format!("{}1{}", "[".repeat(200), "]".repeat(200));
    for text in [
        r#"{"unterminated": 1"#,
        "{not: json}",
        "[1, 2,]",
        r#"{"a": 1} trailing words"#,
        deep.as_str(),
    ] {
        let cell = mcp(vec![text_item(text)]);
        let expanded = super::text(&cell.transcript_lines(u16::MAX));
        assert_eq!(
            expanded.lines().nth(1),
            Some(format!("  └ {text}").as_str()),
            "{text}"
        );
    }
    let large = format!("[{}0]", "1234567890,".repeat(100_000));
    let cell = mcp(vec![text_item(&large)]);
    assert!(cell.transcript_lines(80).len() > 1000);
    assert!(raw(&cell).ends_with("1234567890,0]"));
}

/// A server's image of megabytes reaches the entry as its type and size.
#[test]
fn a_multi_megabyte_image_is_one_summary_row() {
    let image = "A".repeat(8 * 1024 * 1024);
    let display = AgentToolDisplay::fallback(&ToolOutput {
        content: vec![ToolOutputContent::Image {
            mime_type: "image/png".to_string(),
            data: image,
            detail: None,
        }],
        structured_content: None,
        is_error: false,
    });
    assert_eq!(
        display,
        AgentToolDisplay::Content {
            blocks: vec![AgentOutputBlock::Image {
                mime_type: "image/png".to_string(),
                bytes: 6 * 1024 * 1024
            }]
        }
    );
    let cell = shown("mcp__shots__take", None, display);
    assert_eq!(
        both(&cell, 80),
        "• Called mcp__shots__take\n  └ image/png · 6.0 MiB\n─── expanded (None) ───\n• Called mcp__shots__take\n  └ image/png · 6.0 MiB"
    );
}

/// Whatever a server writes in a summary's fields stays on one row
/// without controls.
#[test]
fn a_hostile_summary_stays_one_row_without_controls() {
    let cell = shown(
        "mcp__x__y",
        None,
        AgentToolDisplay::Content {
            blocks: vec![
                AgentOutputBlock::Image {
                    mime_type: "image/png\n\u{1b}[2Jcleared".to_string(),
                    bytes: 1,
                },
                AgentOutputBlock::Link {
                    name: "x".to_string(),
                    uri: "https://e.x/\r\u{7}\u{1b}]8;;evil\u{7}a".to_string(),
                    mime_type: None,
                },
                AgentOutputBlock::Text {
                    text: "{\"k\": \"\\u001b[31m\"}\u{1b}[31m".to_string(),
                },
            ],
        },
    );
    let expanded = super::text(&cell.transcript_lines(80));
    assert_eq!(
        expanded,
        "• Called mcp__x__y\n  └ image/png cleared · 1 B\n    link: https://e.x/ a\n    {\"k\": \"\\u001b[31m\"}"
    );
    assert!(!expanded.contains('\u{1b}'));
}

/// No row wider than the entry, at any width, a wide character counted
/// as two columns.
#[test]
fn rows_fit_any_width_with_wide_characters() {
    let cell = mcp(vec![
        text_item(r#"{"标题":"终端渲染的宽字符测试","内容":["你好世界你好世界你好世界",42]}"#),
        text_item("一行很长的中文文本，用来测试在窄终端中是否会按显示宽度正确换行而不越界。"),
        ToolOutputContent::ResourceLink {
            name: "资源".to_string(),
            uri: "file:///路径/文件名很长很长很长.txt".to_string(),
            mime_type: None,
        },
    ]);
    for width in [120, 80, 40, 20, 12] {
        for line in cell
            .display_lines(width)
            .iter()
            .chain(&cell.transcript_lines(width))
        {
            assert!(line_width(line) <= usize::from(width), "{width}: {line:?}");
        }
    }
}

/// An MCP result's structured content shows as the items that repeat it.
#[test]
fn structured_mcp_content_shows_its_items() {
    let output = ToolOutput {
        content: vec![text_item(r#"{"temperature":21,"unit":"C"}"#)],
        structured_content: Some(json!({ "temperature": 21, "unit": "C" })),
        is_error: false,
    };
    let cell = shown(
        "mcp__weather__now",
        None,
        AgentToolDisplay::fallback(&output),
    );
    assert_eq!(
        render(&cell, 80),
        "• Called mcp__weather__now\n  └ {\"temperature\": 21, \"unit\": \"C\"}"
    );
}
