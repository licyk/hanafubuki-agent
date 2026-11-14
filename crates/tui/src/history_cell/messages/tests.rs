// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The answer's source-backed cell: Codex's render cache tests, and the
//! cell laid out for each width it is shown at.

use ratatui::text::Line;

use super::*;
use crate::history_cell::HistoryRenderMode;

fn text(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| line.to_string().trim_end().to_string())
        .collect()
}

const TABLE: &str = "| Step | Owner | State | Why it matters |\n|---|---|---|---|\n| Parse the config | parser | done | Every later step reads what the parser keeps, so it goes first. |\n| 第二步：校验 | checker | failed | 一个很长的说明，在窄的列里会换行，读起来很费劲。 |\n| Write the report | writer | pending | The person reads this one, at the end of the run. |\n";

#[test]
fn finalized_markdown_reuses_rendered_lines() {
    let cell = AgentMarkdownCell::new("finalized **markdown**".to_string(), "/tmp".to_string());
    let width = 48;
    let rendered = cell.display_lines(width);
    assert_eq!(text(&rendered), vec!["• finalized markdown"]);
    cell.rendered
        .replace_cached_lines(vec![Line::from("cached").into()]);
    assert_eq!(cell.display_lines(width), vec![Line::from("cached")]);
}

#[test]
fn finalized_markdown_cache_misses_when_width_or_list_spacing_changes() {
    let cell = AgentMarkdownCell::new("finalized **markdown**".to_string(), "/tmp".to_string());
    let expected = cell.display_lines(48);
    cell.rendered
        .replace_cached_lines(vec![Line::from("cached").into()]);
    cell.display_lines(47);
    assert_eq!(cell.display_lines(48), expected);
    cell.rendered
        .replace_cached_lines(vec![Line::from("cached").into()]);
    cell.retained_lines(48, false);
    assert_eq!(cell.display_lines(48), expected);
}

#[test]
fn raw_markdown_bypasses_the_rich_render_cache() {
    let source = "finalized **markdown**";
    let cell = AgentMarkdownCell::new(source.to_string(), "/tmp".to_string());
    cell.display_lines(48);
    cell.rendered
        .replace_cached_lines(vec![Line::from("cached").into()]);
    assert_eq!(
        cell.display_lines_for_mode(48, HistoryRenderMode::Raw),
        vec![Line::from(source)]
    );
}

/// A table is laid out again for each width: a grid where its columns
/// read, key/value records where they do not.
#[test]
fn a_table_is_laid_out_again_for_the_width_it_is_shown_at() {
    let cell = AgentMarkdownCell::new(TABLE.to_string(), "/tmp".to_string());
    let wide = text(&cell.display_lines(100));
    assert!(wide.iter().any(|line| line.contains("━")), "{wide:#?}");
    let narrow = text(&cell.display_lines(40));
    assert!(!narrow.iter().any(|line| line.contains("━")), "{narrow:#?}");
    assert!(
        narrow
            .iter()
            .filter(|line| line.trim() == "Why it matters")
            .count()
            == 3,
        "{narrow:#?}"
    );
    // And back: the same rows as at first.
    assert_eq!(text(&cell.display_lines(100)), wide);
}

/// Every row within the width from 20 columns up; narrower, no panic and
/// rows at most a few columns over (M1's bound for a table's stacked
/// records), which the views wrap.
#[test]
fn narrow_widths_neither_overflow_nor_panic() {
    let source = format!("Intro with `code` and 中文.\n\n{TABLE}\n- one\n- two that wraps here\n\n```rust\nfn main() {{}}\n```\n");
    let cell = AgentMarkdownCell::new(source, "/tmp".to_string());
    for width in 0..=120u16 {
        for lines in [cell.display_lines(width), cell.retained_lines(width, false)] {
            let widest = lines.iter().map(Line::width).max().unwrap_or(0);
            let allowed = if width >= 20 {
                usize::from(width)
            } else {
                usize::from(width) + 4
            };
            assert!(
                widest <= allowed.max(2),
                "width {width}: {widest} columns\n{:#?}",
                text(&lines)
            );
        }
    }
}

#[test]
fn the_retained_presentation_spaces_lists_uniformly() {
    let source = "- One\n- This item has enough words to wrap at this width\n- Three\n";
    let cell = AgentMarkdownCell::new(source.to_string(), "/tmp".to_string());
    assert_eq!(
        text(&cell.display_lines(30)),
        vec![
            "• • One",
            "  • This item has enough words",
            "    to wrap at this width",
            "",
            "  • Three",
        ]
    );
    assert_eq!(
        text(&cell.retained_lines(30, false)),
        vec![
            "• • One",
            "",
            "  • This item has enough words",
            "    to wrap at this width",
            "",
            "  • Three",
        ]
    );
}

#[test]
fn a_stream_span_takes_in_what_was_inserted_among_its_cells() {
    let stream = StreamId::next();
    let other = StreamId::next();
    let row = |stream: StreamId, first: bool| -> Arc<dyn HistoryCell> {
        Arc::new(AgentMessageCell::new(
            vec![Line::from("row").into()],
            first,
            stream,
        ))
    };
    let note: Arc<dyn HistoryCell> = Arc::new(crate::history_cell::new_info_event(
        "note".to_string(),
        None,
    ));
    let mut cells = vec![
        row(other, true),
        row(stream, true),
        Arc::clone(&note),
        row(stream, false),
    ];
    let span = stream_span(&cells, stream).unwrap();
    assert_eq!(span, 1..4);
    let consolidated: Arc<dyn HistoryCell> = Arc::new(AgentMarkdownCell::new(
        "row\nrow".to_string(),
        "/".to_string(),
    ));
    splice_stream(&mut cells, span, stream, Arc::clone(&consolidated));
    assert_eq!(cells.len(), 3);
    assert!(Arc::ptr_eq(&cells[1], &consolidated));
    assert!(Arc::ptr_eq(&cells[2], &note));
    assert_eq!(cells[0].stream_id(), Some(other));
    assert_eq!(stream_span(&cells, stream), None);
}

/// A display formula of several rows is laid out again for each width:
/// whole, its rows never split, where it fits with four columns to spare;
/// as its source, wrapped like text, below that. An answer renders two
/// columns narrower than the cell and a plan four.
#[test]
fn a_display_formula_is_whole_or_its_source_at_every_width() {
    let source = "Roots:\n\n$$\nx=\\frac{-b\\pm\\sqrt{b^2-4ac}}{2a}\n$$\n\nDone.\n";
    let layout = ["  -b±√(b²-4ac)", "x=────────────", "       2a"];
    let answer =
        |width: u16| text(&AgentMarkdownCell::new(source.into(), "/".into()).display_lines(width));
    let plan = |width: u16| {
        text(
            &crate::history_cell::ProposedPlanCell::new(source.into(), "/".into())
                .display_lines(width),
        )
    };
    for (cell, indent, threshold) in [
        (&answer as &dyn Fn(u16) -> Vec<String>, 2, 20),
        (&plan, 2, 22),
    ] {
        for width in 8..=60u16 {
            let rows = cell(width);
            let laid_out = layout.iter().all(|row| {
                rows.iter()
                    .any(|shown| shown.get(indent..) == Some(row.trim_end()))
            });
            assert_eq!(laid_out, width >= threshold, "{width}: {rows:?}");
            assert_eq!(
                rows.iter().any(|row| row.trim() == "$$"),
                width < threshold,
                "{width}: {rows:?}"
            );
            // A plan's header row is not wrapped.
            assert!(
                rows.iter()
                    .filter(|row| *row != "• Proposed Plan")
                    .all(|row| crate::wrapping::display_width(row) <= usize::from(width)),
                "{width}: {rows:?}"
            );
        }
    }
}

/// Codex's interruption snapshots at 20 columns (`answer_math_narrow`,
/// `plan_math_narrow`), the cells' part: an answer and a plan cut inside
/// an open display show its source, wrapped as Codex wraps it.
#[test]
fn an_open_display_at_twenty_columns_is_codexs() {
    let source = "Intro.\n\n$$\n\\frac{a+b+c+d+e+f}{g+h}\n";
    assert_eq!(
        text(&AgentMarkdownCell::new(source.into(), "/".into()).display_lines(20)),
        ["• Intro.", "", "  $$", "  \\frac{a+b+c+d+e+f}", "  {g+h}"]
    );
    assert_eq!(
        text(
            &crate::history_cell::ProposedPlanCell::new(source.into(), "/".into())
                .display_lines(20)
        ),
        [
            "• Proposed Plan",
            "",
            "",
            "  Intro.",
            "",
            "  $$",
            "  \\frac{a+b+c+d+e+",
            "  f}{g+h}",
            ""
        ]
    );
}
