// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

mod equality;
mod links;
mod math;
mod mermaid;
mod preview;
mod resize;
mod visible;

use super::*;

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

fn stream_controller(width: Option<usize>) -> StreamController {
    StreamController::new(width, "/repo".to_string(), ListSpacing::AfterMultiline)
}

fn plan_stream_controller(width: Option<usize>) -> PlanStreamController {
    PlanStreamController::new(width, "/repo".to_string(), ListSpacing::AfterMultiline)
}

fn full_render(source: &str, width: Option<usize>) -> Vec<String> {
    let mut rendered = Vec::new();
    crate::markdown::append_markdown_agent(source, width, Some("/repo"), &mut rendered);
    lines_to_plain_strings(&rendered)
}

/// The cell a stream's end committed its last rows in.
fn finalized_cell(finalized: &Finalized) -> Option<&dyn HistoryCell> {
    finalized
        .update
        .as_ref()
        .and_then(|update| update.committed.as_deref())
}

/// The answer's rows, a tick after each delta until the queue is empty,
/// then its end; without the two columns of indent.
fn collect_streamed_lines(deltas: &[&str], width: Option<usize>) -> Vec<String> {
    let mut ctrl = stream_controller(width);
    let mut lines = Vec::new();
    for delta in deltas {
        ctrl.push(delta);
        while let (Some(cell), idle) = ctrl.on_commit_tick() {
            lines.extend(cell.display_lines(u16::MAX));
            if idle {
                break;
            }
        }
    }
    let finalized = ctrl.finalize();
    if let Some(cell) = finalized_cell(&finalized) {
        lines.extend(cell.display_lines(u16::MAX));
    }
    lines_to_plain_strings(&lines)
        .into_iter()
        .map(|line| line.chars().skip(2).collect::<String>())
        .collect()
}

fn collect_plan_streamed_lines(deltas: &[&str], width: Option<usize>) -> Vec<String> {
    let mut ctrl = plan_stream_controller(width);
    let mut lines = Vec::new();
    for delta in deltas {
        ctrl.push(delta);
        while let (Some(cell), idle) = ctrl.on_commit_tick() {
            lines.extend(cell.display_lines(u16::MAX));
            if idle {
                break;
            }
        }
    }
    let finalized = ctrl.finalize();
    if let Some(cell) = finalized_cell(&finalized) {
        lines.extend(cell.display_lines(u16::MAX));
    }
    lines_to_plain_strings(&lines)
}

#[test]
fn queued_heading_is_emitted_once_after_incremental_append() {
    let mut ctrl = stream_controller(Some(80));
    assert!(ctrl.push("Paragraph.\n\n# Heading\n\n"));
    ctrl.push("Next paragraph.\n");

    let (cell, _) = ctrl.on_commit_tick_batch(usize::MAX);
    let mut streamed = cell
        .into_iter()
        .flat_map(|cell| cell.display_lines(u16::MAX))
        .collect::<Vec<_>>();
    if let Some(cell) = finalized_cell(&ctrl.finalize()) {
        streamed.extend(cell.display_lines(u16::MAX));
    }
    let streamed = lines_to_plain_strings(&streamed);
    assert_eq!(
        streamed
            .iter()
            .filter(|line| line.contains("# Heading"))
            .count(),
        1,
        "expected the streamed heading to be emitted once: {streamed:?}",
    );
}

#[test]
fn controller_tick_batch_zero_is_noop() {
    let mut ctrl = stream_controller(Some(80));
    assert!(ctrl.push("line one\n"));
    assert_eq!(ctrl.queued_lines(), 1);

    let (cell, idle) = ctrl.on_commit_tick_batch(0);
    assert!(cell.is_none(), "batch size 0 should not emit lines");
    assert!(!idle, "batch size 0 should not drain queued lines");
    assert_eq!(ctrl.queued_lines(), 1);
}

#[test]
fn controller_loose_vs_tight_with_commit_ticks_matches_full() {
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
    let streamed = collect_streamed_lines(&deltas, None);
    let source: String = deltas.iter().copied().collect();
    assert_eq!(streamed, full_render(&source, None));
    assert_eq!(
        streamed,
        vec![
            "Loose vs. tight list items:",
            "",
            "1. Tight item",
            "2. Another tight item",
            "3. Loose item with its own paragraph.",
            "",
            "   This paragraph belongs to the same list item.",
            "",
            "4. Second loose item with a nested list after a blank line.",
            "    • Nested bullet under a loose item",
            "    • Another nested bullet",
        ]
    );
}

#[test]
fn controller_keeps_pre_table_lines_queued_when_table_is_confirmed() {
    let mut ctrl = stream_controller(Some(80));

    ctrl.push("Intro line before table.\n");
    assert_eq!(ctrl.queued_lines(), 1);

    ctrl.push("| Key | Value |\n");
    ctrl.push("| --- | --- |\n");
    assert_eq!(
        ctrl.queued_lines(),
        1,
        "pre-table line should remain queued after table confirmation",
    );

    let (cell, idle) = ctrl.on_commit_tick();
    let committed = cell
        .map(|cell| lines_to_plain_strings(&cell.display_lines(u16::MAX)))
        .unwrap_or_default();
    assert!(committed
        .iter()
        .any(|line| line.contains("Intro line before table.")));
    assert!(idle, "only pre-table content should have been queued");

    ctrl.push(
        "| first | row |\n\nAfter table.\n\n| Next | Table |\n| --- | --- |\n| second | row |\n",
    );
    let (cell, idle) = ctrl.on_commit_tick_batch(usize::MAX);
    let committed = lines_to_plain_strings(
        &cell
            .expect("completed table and prose should be committed")
            .display_lines(u16::MAX),
    );
    assert!(idle);
    assert!(ctrl.has_live_tail(), "the later table must stay mutable");
    assert_eq!(
        committed,
        vec![
            "  ",
            "   Key      Value",
            "  ━━━━━━━  ━━━━━━━",
            "   first    row",
            "  ",
            "  After table.",
        ]
    );
}

#[test]
fn a_late_reference_definition_marks_the_stream_for_repair() {
    let mut ctrl = stream_controller(Some(80));
    for delta in [
        "| Key | Value |\n| --- | --- |\n| [A][ref] | row |\n",
        "\nAfter table.\n\n",
        "[ref]: https://example.com\n",
    ] {
        ctrl.push(delta);
        ctrl.on_commit_tick_batch(usize::MAX);
    }
    assert!(
        !ctrl.has_live_tail(),
        "the completed table should be released"
    );
    let finalized = ctrl.finalize();
    assert!(finalized.consolidation.expect("consolidated").repair);
}

#[test]
fn controller_does_not_hold_back_pipe_prose_without_table_delimiter() {
    let mut ctrl = stream_controller(Some(80));
    ctrl.push("status | owner | note\n");
    let (_first_commit, first_idle) = ctrl.on_commit_tick();
    assert!(first_idle);
    ctrl.push("next line\n");
    let (second_commit, _second_idle) = ctrl.on_commit_tick();
    assert!(
        second_commit.is_some(),
        "expected prose lines to be released once no table delimiter follows"
    );
}

#[test]
fn controller_does_not_stall_repeated_pipe_prose_paragraphs() {
    let mut ctrl = stream_controller(Some(80));
    ctrl.push("alpha | beta\n\n");
    let (_first_commit, first_idle) = ctrl.on_commit_tick();
    assert!(first_idle);
    ctrl.push("gamma | delta\n\n");
    let (second_commit, _second_idle) = ctrl.on_commit_tick();
    let second_lines = second_commit
        .map(|cell| lines_to_plain_strings(&cell.display_lines(u16::MAX)))
        .unwrap_or_default();
    assert!(
        second_lines
            .iter()
            .any(|line| line.contains("alpha | beta")),
        "expected the first pipe-prose paragraph to stream before finalize; got {second_lines:?}",
    );
}

#[test]
fn controller_handles_table_immediately_after_heading() {
    let deltas = vec![
        "### 1) Basic table\n",
        "| Name | Role | Status |\n",
        "|---|---|---|\n",
        "| Alice | Admin | Active |\n",
        "| Bob | Editor | Pending |\n",
        "\n",
    ];
    let source: String = deltas.iter().copied().collect();
    assert_eq!(
        collect_streamed_lines(&deltas, Some(100)),
        full_render(&source, Some(100))
    );
}

#[test]
fn controller_renders_separators_for_no_outer_pipes_table_shape() {
    let source = "### 1) Basic\n\n| Name | Role | Active |\n|---|---|---|\n| Alice | Engineer | Yes |\n| Bob | Designer | No |\n\n### 2) No outer pipes\n\nCol A | Col B | Col C\n--- | --- | ---\nx | y | z\n10 | 20 | 30\n\n### 3) Another table\n\n| Key | Value |\n|---|---|\n| a | b |\n";
    let deltas = source.split_inclusive('\n').collect::<Vec<_>>();
    let streamed = collect_streamed_lines(&deltas, Some(100));
    assert_eq!(streamed, full_render(source, Some(100)));
    assert!(!streamed
        .iter()
        .any(|line| line.trim() == "Col A | Col B | Col C"));
    assert!(streamed.iter().any(|line| line.contains('━')));
}

#[test]
fn controller_stabilizes_two_column_no_outer_table_in_response() {
    let deltas = vec![
        "A | B\n",
        "--- | ---\n",
        "left | right\n",
        "\n",
        "After table paragraph.\n",
    ];
    let source: String = deltas.iter().copied().collect();
    let streamed = collect_streamed_lines(&deltas, Some(80));
    assert_eq!(streamed, full_render(&source, Some(80)));
    assert!(streamed.iter().any(|line| line.contains('━')));
}

#[test]
fn controller_keeps_markdown_fenced_tables_mutable_until_finalize() {
    let source = "```md\n| A | B |\n|---|---|\n| 1 | 2 |\n```\n";
    let deltas = source.split_inclusive('\n').collect::<Vec<_>>();
    let streamed = collect_streamed_lines(&deltas, Some(80));
    assert_eq!(streamed, full_render(source, Some(80)));
    assert!(streamed.iter().any(|line| line.contains('━')));
    assert!(!streamed.iter().any(|line| line.trim() == "| A | B |"));
}

#[test]
fn controller_keeps_non_markdown_fenced_tables_as_code() {
    let source = "```sh\n| A | B |\n|---|---|\n| 1 | 2 |\n```\n";
    let deltas = source.split_inclusive('\n').collect::<Vec<_>>();
    let streamed = collect_streamed_lines(&deltas, Some(80));
    assert_eq!(streamed, full_render(source, Some(80)));
    assert!(streamed.iter().any(|line| line.trim() == "| A | B |"));
    assert!(!streamed
        .iter()
        .any(|line| line.contains('━') || line.contains('─')));
}

#[test]
fn controller_live_view_matches_render_during_interleaved_table_streaming() {
    let source = "Project updates are easier to scan when narrative and structured data alternate.\n\n| Focus Area | Owner | Priority | Status |\n|---|---|---|---|\n| Authentication cleanup | Maya | High | 80% |\n| CLI error messages | Jordan | Medium | 55% |\n| Docs refresh | Lee | Low | 30% |\n\nThe first checkpoint shows progress, but we still have open risks.\n\n| Task | Command / Artifact | Due | State |\n|---|---|---|---|\n| Run unit tests | `cargo test -p core` | Today | ✅ |\n| Snapshot review | `cargo insta pending-snapshots` | Today | ⏳ |\n| Changelog draft | Release template (https://example.com/) | Tomorrow | 📝 |\n\nFinal sign-off criteria are summarized below.\n";
    let width = Some(72usize);
    let mut ctrl = stream_controller(width);
    let mut emitted = Vec::new();
    for delta in source.split_inclusive('\n') {
        ctrl.push(delta);
        loop {
            let (cell, idle) = ctrl.on_commit_tick();
            if let Some(cell) = cell {
                emitted.extend(
                    lines_to_plain_strings(&cell.display_lines(u16::MAX))
                        .into_iter()
                        .map(|line| line.chars().skip(2).collect::<String>()),
                );
            }
            if idle {
                break;
            }
        }
        let mut visible = emitted.clone();
        visible.extend(lines_to_plain_strings(
            &crate::terminal_hyperlinks::visible_lines(ctrl.tail_lines()),
        ));
        assert_eq!(
            visible,
            full_render(ctrl.committed_source(), width),
            "live view diverged after delta: {delta:?}"
        );
    }
    assert!(
        !ctrl.has_live_tail(),
        "completed tables and prose should be committed before the end",
    );
}

#[test]
fn plan_controller_streamed_table_matches_final_render() {
    let deltas = vec![
        "## Build plan\n\n",
        "| Step | Owner |\n",
        "|---|---|\n",
        "| Write tests | Agent |\n",
        "| Verify output | User |\n",
        "\n",
    ];
    let streamed = collect_plan_streamed_lines(&deltas, Some(80));
    let source: String = deltas.iter().copied().collect();
    let baseline = collect_plan_streamed_lines(&[source.as_str()], Some(80));
    assert_eq!(streamed, baseline);
    assert!(streamed.iter().any(|line| line.contains('━')));
    assert!(!streamed
        .iter()
        .any(|line| line.trim() == "| Step | Owner |"));
}

#[test]
fn plan_controller_holds_table_header_as_live_tail() {
    let mut ctrl = plan_stream_controller(Some(80));
    assert!(ctrl.push("Intro\n"));
    let (_cell, idle) = ctrl.on_commit_tick_batch(usize::MAX);
    assert!(idle, "intro line should fully drain");
    ctrl.push("| Step | Owner |\n");
    assert_eq!(ctrl.queued_lines(), 0);
    let update = ctrl.update(None).expect("the tail shows the header");
    let shown = update.shown.and_then(|rows| rows.0).expect("tail rows");
    assert!(lines_to_plain_strings(&shown.display_lines(80))
        .iter()
        .any(|line| line.contains("Step")));
}

/// The rows a stream at `width` commits, its cells shown at `width`, and
/// what its end gives.
fn stream_at(source: &str, width: u16, plan: bool) -> (Vec<String>, Finalized) {
    let mut lines = Vec::new();
    let finalized = if plan {
        let mut ctrl = PlanStreamController::new(
            Some(usize::from(width) - 4),
            "/project".to_string(),
            ListSpacing::AfterMultiline,
        );
        for chunk in source.split_inclusive('\n') {
            ctrl.push(chunk);
            while let (Some(cell), _) = ctrl.on_commit_tick() {
                lines.extend(cell.display_lines(width));
            }
        }
        ctrl.finalize()
    } else {
        let mut ctrl = StreamController::new(
            Some(usize::from(width) - 2),
            "/project".to_string(),
            ListSpacing::AfterMultiline,
        );
        for chunk in source.split_inclusive('\n') {
            ctrl.push(chunk);
            while let (Some(cell), _) = ctrl.on_commit_tick() {
                lines.extend(cell.display_lines(width));
            }
        }
        ctrl.finalize()
    };
    if let Some(cell) = finalized_cell(&finalized) {
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
            assert!(!consolidation.repair, "plan {plan}, width {width}");
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
fn an_answer_that_received_nothing_is_not_consolidated() {
    let mut ctrl = stream_controller(Some(78));
    ctrl.push("");
    assert!(ctrl.update(None).is_none());
    let finalized = ctrl.finalize();
    assert!(finalized_cell(&finalized).is_none());
    assert!(finalized.consolidation.is_none());
}
