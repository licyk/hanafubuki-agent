// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Pipe tables: Codex's grid, its key/value fallback, and their widths.

use insta::assert_snapshot;
use ratatui::style::Modifier;
use ratatui::text::Text;
use unicode_width::UnicodeWidthStr;

use super::plain_lines;
use crate::markdown_render::{
    render_markdown_text, render_markdown_text_with_width, render_markdown_text_with_width_and_cwd,
};

#[test]
fn table_renders_app_style_rows_with_themed_bold_header() {
    let md = "| A | B |\n|---|---|\n| 1 | 2 |\n";
    let text = render_markdown_text(md);
    assert_eq!(
        plain_lines(&text),
        vec![" A      B", "━━━━━  ━━━━━", " 1      2"]
    );
    assert!(text.lines[0].style.add_modifier.contains(Modifier::BOLD));
    assert!(
        text.lines[0].style.fg.is_some(),
        "expected the syntax theme to provide a table header accent"
    );
    assert!(text.lines[1].spans[0]
        .style
        .add_modifier
        .contains(Modifier::DIM));
    assert!(!text.lines[2].style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn table_alignment_respects_markers() {
    let md = "| Left | Center | Right |\n|:-----|:------:|------:|\n| a | b | c |\n";
    let lines = plain_lines(&render_markdown_text(md));
    assert_eq!(lines[0], " Left    Center    Right");
    assert_eq!(lines[2], " a         b           c");
}

#[test]
fn table_separates_logical_rows_after_wrapped_content() {
    let md = "| Key | Description |\n| --- | --- |\n| -v | Enable very verbose logging output for debugging |\n| -q | Quiet output |\n";
    let lines = plain_lines(&render_markdown_text_with_width(md, Some(30)));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Enable very verbose"))
            && lines.iter().any(|line| line.contains("logging output")),
        "expected wrapped row content: {lines:?}"
    );
    let separator_indices: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            ((line.contains('━') || line.contains('─'))
                && line.chars().all(|ch| matches!(ch, '━' | '─' | ' ')))
            .then_some(idx)
        })
        .collect();
    let wrapped_row_end = lines
        .iter()
        .position(|line| line.contains("logging output"))
        .expect("expected final wrapped line");
    assert_eq!(separator_indices.len(), 2);
    assert!(separator_indices[1] > wrapped_row_end);
    assert!(!lines
        .last()
        .is_some_and(|line| line.contains('━') || line.contains('─')));
}

#[test]
fn table_wraps_file_paths_before_collapsing_narrative_columns_snapshot() {
    let md = r#"| Unit | Files | Adds | Removes | What It Adds |
|---|---:|---:|---:|---|
| Suggestion engine and unit coverage | [next_prompt_suggestion.rs](/Users/example/code/codex/codex-rs/core/src/next_prompt_suggestion.rs:1), [next_prompt_suggestion_tests.rs](/Users/example/code/codex/codex-rs/core/src/next_prompt_suggestion_tests.rs:1) | 704 | 0 | Sampling workflow, stable-history checks, tool-flow suppression, fast reasoning profile, filtering rules, cancellation and timeout. |
| Model instruction fragment and contextual isolation | [next_prompt_suggestion.rs](/Users/example/code/codex/codex-rs/core/src/context/next_prompt_suggestion.rs:1), [contextual_user_message_tests.rs](/Users/example/code/codex/codex-rs/core/src/context/contextual_user_message_tests.rs:1) | 54 | 0 | Synthetic suggestion prompt and an isolation test for ordinary user text. |
"#;
    let text =
        render_markdown_text_with_width_and_cwd(md, Some(120), Some("/Users/example/code/codex"));
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn table_renders_stacked_key_value_records_when_path_column_becomes_too_narrow_snapshot() {
    let md = r#"| Session | Why useful | Detected table blocks |
| --- | --- | --- |
| [2026-05-25 current gallery](/Users/felipe.coury/.codex/sessions/2026/05/25/rollout-2026-05-25T18-13-09-019e60fc-0518-7c21-9596-980fe97225ba.jsonl) | The large gallery from this thread: emojis, links, emphasis, code, alignment, paragraphs, and a 30+ row table | 7 |
| [2026-05-14 renderer testing](/Users/felipe.coury/.codex/sessions/2026/05/14/rollout-2026-05-14T12-57-18-019e2734-e500-7011-8278-975c94d06000.jsonl) | Explicit "markdown tables for testing" session with several successive assistant samples | 16 |
| [2026-05-14 five-table test](/Users/felipe.coury/.codex/sessions/2026/05/14/rollout-2026-05-14T12-27-57-019e271a-064c-78c3-a5cd-a6f20a0c1ad5.jsonl) | Explicit request for five tables containing emojis, code, italics, and varied cell content | 10 |
"#;
    let text = render_markdown_text_with_width_and_cwd(md, Some(42), None);
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn table_renders_records_when_multiple_prose_columns_are_starved_snapshot() {
    let md = r#"| Issue | Activity | Complexity | Why start |
| --- | ---: | ---: | --- |
| [#24485: newline shortcut fails in PyCharm terminal on Windows](https://github.com/openai/codex/issues/24485) | `+1` 0, substantive comments 0 | Low | New, deterministic regression range; localized composer/keymap path. |
| [#23926: Vim composer `e` stalls at word end](https://github.com/openai/codex/issues/23926) | `+1` 0, comments 0 | Low | Standing best quick win; deterministic motion bug. |
| [#23651: Zellij scrollback misses Codex transcript over SSH](https://github.com/openai/codex/issues/23651) | `+1` 3, human comments 2 | Medium | Clear regression and strong scrollback evidence. |
| [#23740: raw ANSI/control sequences in Windows Terminal](https://github.com/openai/codex/issues/23740) | `+1` 7, human comments 7 | Medium | Highest activity; established Windows rendering regression family. |
| [#24527: typing lag increases with session length](https://github.com/openai/codex/issues/24527) | `+1` 0, substantive comments 0 | Medium | New TUI-visible performance report; needs profiling before implementation. |
"#;
    let text = render_markdown_text_with_width(md, Some(76));
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn table_keeps_grid_when_only_one_compact_record_fragments_snapshot() {
    let md = r#"| Key | Date | State |
| --- | --- | --- |
| short | 2025-01-01 | Ready |
| verylongidentifier | 2025-02-02 | Ready |
| final | 2025-03-03 | Done |
"#;
    let text = render_markdown_text_with_width(md, Some(40));
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn table_renders_key_value_records_when_compact_fragmentation_is_systemic_snapshot() {
    let md = r#"| Key | Notes |
| --- | --- |
| firstlongid | A readable explanatory sentence for this row. |
| secondlongid | Another readable explanatory sentence for this row. |
| short | A final readable explanatory sentence for this row. |
"#;
    let text = render_markdown_text_with_width(md, Some(17));
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn table_renders_halfwidth_sound_marks_at_constrained_width_snapshot() {
    let md = r#"| Key | Notes |
| --- | --- |
| ｶﾞﾊﾟtail | First ｶﾞ row with an escaped \| pipe. |
| ﾊﾟｶﾞtail | Second ﾊﾟ row with an escaped \| pipe. |
| short | Final ｶﾞﾊﾟ row. |
"#;
    let grid = render_markdown_text_with_width(md, Some(23));
    let records = render_markdown_text_with_width(md, Some(17));
    assert_snapshot!(format!(
        "grid (23 cells):\n{}\n\nrecords (17 cells):\n{}",
        plain_lines(&grid).join("\n"),
        plain_lines(&records).join("\n")
    ));
}

#[test]
fn table_inside_blockquote_has_quote_prefix() {
    let md = "> | A | B |\n> |---|---|\n> | 1 | 2 |\n";
    let lines = plain_lines(&render_markdown_text(md));
    assert!(lines.iter().all(|line| line.starts_with("> ")), "{lines:?}");
    assert!(lines.iter().any(|line| line.contains("━━━━━  ━━━━━")));
}

#[test]
fn escaped_pipes_render_in_table_cells() {
    let md = "| Col |\n| --- |\n| a \\| b |\n";
    let lines = plain_lines(&render_markdown_text(md));
    assert!(lines.iter().any(|line| line.contains("a | b")), "{lines:?}");
}

#[test]
fn table_falls_back_to_key_value_records_if_grid_cannot_fit() {
    let md = "| c1 | c2 | c3 | c4 | c5 | c6 | c7 | c8 | c9 | c10 |\n|---|---|---|---|---|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 |\n";
    let lines = plain_lines(&render_markdown_text_with_width(md, Some(20)));
    assert!(lines.first().is_some_and(|line| line.contains("c1")));
    assert!(lines
        .iter()
        .any(|line| line.contains("c10") && line.contains("10")));
    assert!(!lines
        .iter()
        .any(|line| line.starts_with('|') || line.contains('━') || line.contains('─')));
}

#[test]
fn table_key_value_fallback_preserves_rich_values_and_themed_labels() {
    let md = "| Key | Content | Extra | More |\n|---|---|---|---|\n| item | [link](https://example.com) | **bold** | `code` |\n";
    let text = render_markdown_text_with_width(md, Some(16));
    let lines = plain_lines(&text);
    for word in ["Key", "item", "link", "bold", "code"] {
        assert!(lines.iter().any(|line| line.contains(word)), "{lines:?}");
    }
    assert!(text.lines[0]
        .spans
        .iter()
        .any(|span| span.content.contains("Key")
            && span.style.add_modifier.contains(Modifier::BOLD)
            && span.style.fg.is_some()));
    assert!(text.lines.iter().any(|line| {
        line.spans
            .iter()
            .any(|span| span.style.add_modifier.contains(Modifier::UNDERLINED))
    }));
}

/// A table at 120, 80, 40 and 20 columns: wide and CJK cells, an empty
/// cell, inline code, emphasis and a link in cells, an alignment row.
#[test]
fn table_width_sweep_snapshot() {
    let md = "| 名前 | Status | Notes |\n|:---|:---:|---:|\n| 東京タワー | ✅ done | The **tower** is `333 m` tall, see [docs](https://example.com/tower) |\n| 🍣 sushi |  | *empty* status above |\n| plain | 🚧 wip | 中文的说明文字，需要换行才能放得下 |\n";
    let stages = [120, 80, 40, 20]
        .map(|width| {
            let text = render_markdown_text_with_width(md, Some(width));
            format!("width {width}:\n{}", plain_lines(&text).join("\n"))
        })
        .join("\n\n");
    assert_snapshot!(stages);
}

/// Samples whose every line must fit the width it was rendered at.
const FITTING_SAMPLES: [&str; 6] = [
    "| A | B |\n|---|---|\n| 1 | 2 |\n",
    "| 名前 | Status | Notes |\n|:---|:---:|---:|\n| 東京タワー | ✅ done | The **tower** is `333 m` tall |\n| 🍣 sushi |  | *empty* |\n",
    "| c1 | c2 | c3 | c4 | c5 | c6 | c7 | c8 | c9 | c10 |\n|---|---|---|---|---|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 |\n",
    "- item\n\n  | A | B |\n  |---|---|\n  | one two three | four |\n",
    "> | Key | Notes |\n> | --- | --- |\n> | firstlongid | A readable explanatory sentence. |\n",
    "| Only header | Second |\n|---|---|\n",
];

#[test]
fn tables_fit_every_width_from_twenty_columns() {
    for source in FITTING_SAMPLES {
        for width in [20, 24, 32, 40, 60, 80, 120] {
            let text = render_markdown_text_with_width(source, Some(width));
            for line in plain_lines(&text) {
                assert!(
                    line.width() <= width,
                    "{width} columns, {} wide: {line:?} in\n{source}",
                    line.width()
                );
            }
        }
    }
}

/// Below twenty columns nothing panics, and a line is wider than the width
/// only by the few columns a label, a value's indent or a quote marker
/// cannot give up.
#[test]
fn tables_at_tiny_widths_do_not_panic_or_overflow_much() {
    for source in FITTING_SAMPLES {
        for width in 0..20 {
            let text = render_markdown_text_with_width(source, Some(width));
            for line in plain_lines(&text) {
                assert!(
                    line.width() <= width.max(1) + 4,
                    "{width} columns, {} wide: {line:?} in\n{source}",
                    line.width()
                );
            }
        }
    }
}

#[test]
fn unbounded_width_keeps_the_natural_grid() {
    let md = "| Wide column header | B |\n|---|---|\n| a value that is fairly long indeed | 2 |\n";
    let lines = plain_lines(&render_markdown_text(md));
    assert_eq!(
        lines,
        vec![
            " Wide column header                    B",
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  ━━━━━",
            " a value that is fairly long indeed    2",
        ]
    );
}

#[test]
fn malformed_tables_snapshot() {
    let samples = [
        // Rows with too few and too many cells.
        "| A | B | C |\n|---|---|---|\n| 1 |\n| 1 | 2 | 3 | 4 |\n",
        // A header with no body.
        "| A | B |\n|---|---|\n",
        // A paragraph swallowed as a row without pipes.
        "| A | B |\n|---|---|\n| 1 | 2 |\nTrailing prose line\n",
        // Two dashes are enough for pulldown-cmark.
        "| A | B |\n|--|--|\n| 1 | 2 |\n",
        // A header and delimiter of different widths is no table.
        "| A | B |\n|---|\n| 1 | 2 |\n",
        // Empty cells everywhere.
        "| | |\n|---|---|\n| | |\n",
    ];
    let rendered = samples
        .map(|source| plain_lines(&render_markdown_text_with_width(source, Some(40))).join("\n"))
        .join("\n---\n");
    assert_snapshot!(rendered);
}

#[test]
fn table_inside_a_list_item_and_after_text_snapshot() {
    let md = "1. Results:\n\n   | Case | Result |\n   |---|---|\n   | first | passed |\n   | second | failed with a long explanation |\n\n2. Next item\n\nIntro line\n| A | B |\n|---|---|\n| 1 | 2 |\n";
    let text = render_markdown_text_with_width(md, Some(36));
    assert_snapshot!(plain_lines(&text).join("\n"));
}

#[test]
fn cells_keep_their_inline_styles() {
    let md = "| Code | Strong |\n|---|---|\n| `x = 1` | **bold** *it* |\n";
    let text: Text = render_markdown_text(md);
    let spans: Vec<_> = text.lines[2].spans.iter().collect();
    let code = spans.iter().find(|span| span.content == "x = 1").unwrap();
    assert_eq!(code.style.fg, Some(ratatui::style::Color::Cyan));
    let bold = spans.iter().find(|span| span.content == "bold").unwrap();
    assert!(bold.style.add_modifier.contains(Modifier::BOLD));
    let italic = spans.iter().find(|span| span.content == "it").unwrap();
    assert!(italic.style.add_modifier.contains(Modifier::ITALIC));
}

#[test]
fn local_links_in_cells_show_their_target() {
    let md = "| File |\n| --- |\n| [lib](/repo/src/lib.rs:12) |\n";
    let text = render_markdown_text_with_width_and_cwd(md, None, Some("/repo"));
    assert!(plain_lines(&text).contains(&" lib (src/lib.rs:12)".to_string()));
}
