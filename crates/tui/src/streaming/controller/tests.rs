// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

fn lines_to_plain_strings(lines: &[ratatui::text::Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect()
}

#[test]
fn controller_loose_vs_tight_with_commit_ticks_matches_full() {
    let mut ctrl = StreamController::new(None, String::new());
    let mut lines = Vec::new();

    // Exact deltas from the session log (section: Loose vs. tight list items)
    let deltas = vec![
        "\n\n",
        "Loose",
        " vs",
        ".",
        " tight",
        " list",
        " items",
        ":\n",
        "1",
        ".",
        " Tight",
        " item",
        "\n",
        "2",
        ".",
        " Another",
        " tight",
        " item",
        "\n\n",
        "1",
        ".",
        " Loose",
        " item",
        " with",
        " its",
        " own",
        " paragraph",
        ".\n\n",
        "  ",
        " This",
        " paragraph",
        " belongs",
        " to",
        " the",
        " same",
        " list",
        " item",
        ".\n\n",
        "2",
        ".",
        " Second",
        " loose",
        " item",
        " with",
        " a",
        " nested",
        " list",
        " after",
        " a",
        " blank",
        " line",
        ".\n\n",
        "  ",
        " -",
        " Nested",
        " bullet",
        " under",
        " a",
        " loose",
        " item",
        "\n",
        "  ",
        " -",
        " Another",
        " nested",
        " bullet",
        "\n\n",
    ];

    // Simulate streaming with a commit tick attempt after each delta.
    for d in deltas.iter() {
        ctrl.push(d);
        while let (Some(cell), idle) = ctrl.on_commit_tick() {
            lines.extend(cell.display_lines(u16::MAX));
            if idle {
                break;
            }
        }
    }
    // Finalize and flush remaining lines now.
    if let Some(cell) = ctrl.finalize().cell {
        lines.extend(cell.display_lines(u16::MAX));
    }

    let streamed: Vec<_> = lines_to_plain_strings(&lines)
        .into_iter()
        // skip • and 2-space indentation
        .map(|s| s.chars().skip(2).collect::<String>())
        .collect();

    // Full render of the same source
    let source: String = deltas.iter().copied().collect();
    let mut rendered: Vec<ratatui::text::Line<'static>> = Vec::new();
    crate::markdown::append_markdown_agent(&source, None, None, &mut rendered);
    let rendered_strs = lines_to_plain_strings(&rendered);

    assert_eq!(streamed, rendered_strs);

    // Also assert exact expected plain strings for clarity.
    let expected = vec![
        "Loose vs. tight list items:".to_string(),
        "".to_string(),
        "1. Tight item".to_string(),
        "2. Another tight item".to_string(),
        "3. Loose item with its own paragraph.".to_string(),
        "".to_string(),
        "   This paragraph belongs to the same list item.".to_string(),
        "".to_string(),
        "4. Second loose item with a nested list after a blank line.".to_string(),
        "    • Nested bullet under a loose item".to_string(),
        "    • Another nested bullet".to_string(),
    ];
    assert_eq!(
        streamed, expected,
        "expected exact rendered lines for loose/tight section"
    );
}

/// The rows a stream at `width` commits, its cells shown at `width`, and
/// what its end gives.
fn stream_at(source: &str, width: u16, plan: bool) -> (Vec<String>, Finalized) {
    let mut lines = Vec::new();
    let finalized = if plan {
        let mut ctrl =
            PlanStreamController::new(Some(usize::from(width) - 4), "/project".to_string());
        for chunk in source.split_inclusive('\n') {
            ctrl.push(chunk);
            while let (Some(cell), _) = ctrl.on_commit_tick() {
                lines.extend(cell.display_lines(width));
            }
        }
        ctrl.finalize()
    } else {
        let mut ctrl = StreamController::new(Some(usize::from(width) - 2), "/project".to_string());
        for chunk in source.split_inclusive('\n') {
            ctrl.push(chunk);
            while let (Some(cell), _) = ctrl.on_commit_tick() {
                lines.extend(cell.display_lines(width));
            }
        }
        ctrl.finalize()
    };
    if let Some(cell) = &finalized.cell {
        lines.extend(cell.display_lines(width));
    }
    (lines_to_plain_strings(&lines), finalized)
}

const TABLE: &str = "Results:\n\n| Case | Result | Notes |\n|---|---|---|\n| first | passed | the parser reads the whole input |\n| second | failed | a long note that wraps in a narrow column |\n\n- [x] Built\n- [ ] Released\n";

/// The consolidated cell shows the stream's rows at the width the stream
/// ran at, so nothing moves when it takes their place; at another width it
/// is laid out for that width, as a stream at that width would have been.
#[test]
fn a_consolidated_stream_keeps_its_rows_and_lays_out_again_at_another_width() {
    for plan in [false, true] {
        for width in [100, 40, 24] {
            let (streamed, finalized) = stream_at(TABLE, width, plan);
            let consolidation = finalized.consolidation.expect("a consolidation");
            let consolidated = lines_to_plain_strings(&consolidation.cell.display_lines(width));
            assert_eq!(consolidated, streamed, "plan {plan}, width {width}");
            for other in [100, 40, 24] {
                let (fresh, _) = stream_at(TABLE, other, plan);
                let relaid = lines_to_plain_strings(&consolidation.cell.display_lines(other));
                assert_eq!(relaid, fresh, "plan {plan}, {width} -> {other}");
            }
        }
    }
}

#[test]
fn an_answer_that_committed_nothing_is_not_consolidated() {
    let mut ctrl = StreamController::new(Some(78), "/project".to_string());
    ctrl.push("");
    let finalized = ctrl.finalize();
    assert!(finalized.cell.is_none());
    assert!(finalized.consolidation.is_none());
}
