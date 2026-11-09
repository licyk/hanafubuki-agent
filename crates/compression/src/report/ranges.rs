// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `renderUncompressedRanges` of `kernel/src/report.ts` of acp-kernel 0.0.189
// at commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use std::cmp::Reverse;

use super::{coverage, format_tokens, StatusReportOptions, StatusSort, Visible};
use crate::model::State;
use crate::number::js_round;
use crate::project::{project_unit, segment_groups, FatalKind, ProjectionContext, UnitRequest};
use crate::protect::compute_protected_refs;
use crate::refs::ordered_ref_pair;

struct Row {
    start_ref: String,
    end_ref: String,
    start_index: usize,
    count: usize,
    tokens: u64,
    /// Tokens per tool, in first-seen order.
    tool_tokens: Vec<(String, u64)>,
}

impl Row {
    fn dominant_tool(&self) -> &str {
        let mut best = ("text", None);
        for (tool, tokens) in &self.tool_tokens {
            if best.1.is_none_or(|best| *tokens > best) {
                best = (tool, Some(*tokens));
            }
        }
        best.0
    }
}

/// The uncompressed history as ranges, segmented as the recommendations
/// are. With a screen, a range the apply step would refuse is flagged, not
/// dropped: the view keeps showing what is visible.
pub(super) fn uncompressed_ranges(
    visible: &[Visible],
    state: &State,
    options: &StatusReportOptions<'_>,
) -> String {
    let total: u64 = visible.iter().map(|message| message.tokens).sum();
    let mut lines = vec![
        format!(
            "UNCOMPRESSED — {} | {} visible messages",
            format_tokens(total),
            visible.len()
        ),
        String::new(),
    ];
    if visible.is_empty() {
        lines.push("  (no uncompressed messages)".to_owned());
        return lines.join("\n");
    }
    let mut rows: Vec<Row> = segment_groups(visible)
        .iter()
        .map(|group| row(group))
        .collect();
    let by_time = options.sort == StatusSort::Time;
    if !by_time {
        rows.sort_by_key(|row| (Reverse(row.tokens), row.start_index));
    }
    let flag = options.screen.map(|(config, original)| {
        let zone = compute_protected_refs(original, state, config);
        (
            ProjectionContext::new(original, state, config),
            coverage(state),
            zone,
        )
    });
    lines.push(format!(
        "Sorted by {}",
        if by_time { "time" } else { "size" }
    ));
    lines.push(String::new());
    for row in rows.iter().take(options.limit) {
        let range = if row.count == 1 {
            row.start_ref.clone()
        } else {
            let (low, high) = ordered_ref_pair(&row.start_ref, &row.end_ref);
            format!("{low}–{high}")
        };
        let average = if row.count > 1 {
            format!(" ({}/msg)", js_round(row.tokens as f64 / row.count as f64))
        } else {
            String::new()
        };
        let flag = flag
            .as_ref()
            .and_then(|(context, coverage, zone)| {
                let request = UnitRequest {
                    start_ref: &row.start_ref,
                    end_ref: &row.end_ref,
                    pre_existing_coverage: coverage,
                    protected_refs: Some(zone),
                };
                not_foldable(project_unit(context, &request, &mut Vec::new()))
            })
            .map_or_else(String::new, |flag| format!("  {flag}"));
        lines.push(format!(
            "  {range}  ({} msgs, {}{average}) {}{flag}",
            row.count,
            format_tokens(row.tokens),
            row.dominant_tool()
        ));
    }
    if rows.len() > options.limit {
        lines.push(format!(
            "  ... and {} more ranges",
            rows.len() - options.limit
        ));
    }
    lines.join("\n")
}

fn row(group: &[Visible]) -> Row {
    let first = &group[0];
    let mut row = Row {
        start_ref: first.r#ref.clone(),
        end_ref: first.r#ref.clone(),
        start_index: first.index,
        count: 0,
        tokens: 0,
        tool_tokens: Vec::new(),
    };
    for message in group {
        row.end_ref = message.r#ref.clone();
        row.count += 1;
        row.tokens += message.tokens;
        match row
            .tool_tokens
            .iter_mut()
            .find(|(tool, _)| *tool == message.tool)
        {
            Some(entry) => entry.1 += message.tokens,
            None => row.tool_tokens.push((message.tool.clone(), message.tokens)),
        }
    }
    row
}

fn not_foldable(
    projection: Result<crate::project::UnitProjection, crate::boundaries::BoundaryError>,
) -> Option<String> {
    let Ok(projection) = projection else {
        return Some("⚠ not foldable: refs unresolvable".to_owned());
    };
    match projection.fatal.as_ref().map(|fatal| fatal.kind) {
        Some(FatalKind::Zone) => Some("⚠ not foldable: protected zone only".to_owned()),
        Some(FatalKind::Split) => {
            let mut parts = Vec::new();
            if projection.split_turn_count > 0 {
                parts.push(format!("{} turn(s)", projection.split_turn_count));
            }
            if projection.split_pair_count > 0 {
                parts.push(format!(
                    "{} tool call/result pair(s)",
                    projection.split_pair_count
                ));
            }
            let what = if parts.is_empty() {
                "turn structure".to_owned()
            } else {
                parts.join(" and ")
            };
            Some(format!("⚠ not foldable: would split {what}"))
        }
        None if projection.filtered_ids.is_empty() && projection.consumed_block_ids.is_empty() => {
            Some("⚠ not foldable: nothing new to fold".to_owned())
        }
        None => None,
    }
}
