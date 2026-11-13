// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/report.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: no host surface line (packs), no
// checkpoint carriers, and tool names in the per-message listing sort by
// their lowercase form instead of the locale's collation.

mod cache;
mod panel;
mod ranges;

use std::cmp::Reverse;
use std::collections::HashSet;

pub use cache::{
    build_cache_report, compute_fold_economics, decompose_sample, format_cache_report,
    summarize_fold_economics, CacheDetail, CacheReport, CacheReportLine, CacheReportOptions,
    CacheSample, CacheTotals, EconomicsSummary, FoldEconomics, FoldEconomicsInput, FoldEvent,
    FormatCacheReportOptions, PriceProfile, SampleDecomposition, FOLD_CLAIM_COLD_ROUNDS,
    FOLD_CLAIM_WARM_HIT_PCT,
};
pub use panel::{
    bar, build_status_panel, cache_hit_stats, format_compact_tokens, format_hit_rate,
    topic_fallback, CacheHitSummary, CacheUsageSample, StatusPanelInput,
};

use crate::model::{Block, Config, Message, Role, State};
use crate::number::{js_round, to_fixed};
use crate::project::SegmentItem;
use crate::refs::BLOCKED_REF;
use crate::render::{count_message_tokens, default_count_tokens};
use crate::text::{clamp_prefix, utf16_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusScope {
    /// The history outside active blocks.
    Uncompressed,
    /// The block ledger.
    Compressed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusView {
    #[default]
    Ranges,
    Messages,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusSort {
    #[default]
    Size,
    Time,
    Tool,
    Age,
}

impl StatusSort {
    fn label(self) -> &'static str {
        match self {
            Self::Size => "size",
            Self::Time => "time",
            Self::Tool => "tool",
            Self::Age => "age",
        }
    }
}

/// What `acp_status` was asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusReportOptions<'a> {
    /// `None` is the overview.
    pub scope: Option<StatusScope>,
    pub view: StatusView,
    /// With the messages view: only this tool's messages.
    pub tool: Option<&'a str>,
    pub sort: StatusSort,
    pub limit: usize,
    /// The configuration and the full history the apply step runs on: with
    /// them, each uncompressed range that would not fold is flagged.
    pub screen: Option<(&'a Config, &'a [Message])>,
}

impl Default for StatusReportOptions<'_> {
    fn default() -> Self {
        Self {
            scope: None,
            view: StatusView::Ranges,
            tool: None,
            sort: StatusSort::Size,
            limit: 30,
            screen: None,
        }
    }
}

/// The `acp_status` text: an overview (the context's breakdown and the
/// active blocks), the uncompressed history as ranges or messages, or the
/// block ledger.
pub fn build_status_report(
    state: &State,
    messages: &[Message],
    options: &StatusReportOptions<'_>,
) -> String {
    let mut active: Vec<&Block> = state.active_blocks().collect();
    active.sort_by_key(|block| block_number(&block.block_id));
    if options.scope == Some(StatusScope::Compressed) {
        return compressed_drilldown(active, options.sort, options.limit);
    }
    let (visible, summary_tokens) = collect_visible(messages, state);
    match (options.scope, options.view) {
        (Some(_), StatusView::Messages) => {
            message_drilldown(&visible, options.tool, options.sort, options.limit)
        }
        (Some(_), StatusView::Ranges) => ranges::uncompressed_ranges(&visible, state, options),
        (None, _) => overview(&visible, summary_tokens, &active, options.limit),
    }
}

pub(crate) fn format_tokens(tokens: u64) -> String {
    if tokens == 0 {
        "0".to_owned()
    } else if tokens >= 1000 {
        format!("{}K", to_fixed(tokens as f64 / 1000.0, 1))
    } else {
        tokens.to_string()
    }
}

fn pct(part: u64, total: u64) -> u64 {
    if part == 0 || total == 0 {
        0
    } else {
        js_round(part as f64 / total as f64 * 100.0) as u64
    }
}

fn block_number(block_id: &str) -> u64 {
    block_id
        .strip_prefix('b')
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}

fn summary_tokens(block: &Block) -> u64 {
    default_count_tokens(&block.summary)
}

/// Summary tokens and counts per tier, when more than one tier is active.
fn tier_breakdown(blocks: &[&Block]) -> Option<String> {
    let mut tokens = [0u64; 3];
    let mut counts = [0usize; 3];
    let mut seen = [false; 3];
    for block in blocks {
        let slot = usize::from(block.tier.clamp(1, 3) - 1);
        tokens[slot] += summary_tokens(block);
        counts[slot] += 1;
        seen[slot] = true;
    }
    if seen.iter().filter(|seen| **seen).count() <= 1 {
        return None;
    }
    let parts: Vec<String> = (0..3)
        .filter(|slot| tokens[*slot] > 0)
        .map(|slot| {
            format!(
                "T{}: {} ({} blocks)",
                slot + 1,
                format_tokens(tokens[slot]),
                counts[slot]
            )
        })
        .collect();
    Some(parts.join(" | "))
}

/// A message with a ref outside every active block.
#[derive(Clone)]
pub(crate) struct Visible {
    pub(crate) r#ref: String,
    pub(crate) tokens: u64,
    pub(crate) tool: String,
    is_tool: bool,
    pub(crate) index: usize,
    is_user: bool,
    gap_before: bool,
}

impl SegmentItem for Visible {
    fn gap_before(&self) -> bool {
        self.gap_before
    }

    fn is_user(&self) -> bool {
        self.is_user
    }
}

/// The visible messages, each with its tool (a result named through its
/// call), and the active blocks' summary tokens. A covered numbered
/// message opens a gap, as it does for the recommendations.
fn collect_visible(messages: &[Message], state: &State) -> (Vec<Visible>, u64) {
    let covered = state.covered_message_ids();
    let summary_tokens = state.active_blocks().map(summary_tokens).sum();
    let call_names: std::collections::HashMap<&str, &str> = messages
        .iter()
        .filter(|message| message.kind == crate::model::Kind::ToolCall)
        .filter_map(|message| {
            Some((
                message.tool_call_id.as_deref()?,
                message.tool_name.as_deref()?,
            ))
        })
        .collect();
    let mut visible = Vec::new();
    let mut pending_gap = false;
    for (index, message) in messages.iter().enumerate() {
        let Some(r#ref) = state.message_refs.by_raw.get(&message.id) else {
            continue;
        };
        let tokens = count_message_tokens(message);
        if covered.contains(message.id.as_str()) || tokens == 0 {
            if r#ref != BLOCKED_REF && tokens > 0 {
                pending_gap = true;
            }
            continue;
        }
        let is_tool = message.is_tool();
        let tool = if is_tool {
            message
                .tool_name
                .as_deref()
                .or_else(|| {
                    message
                        .tool_call_id
                        .as_deref()
                        .and_then(|call_id| call_names.get(call_id).copied())
                })
                .unwrap_or("tool")
        } else {
            "text"
        };
        visible.push(Visible {
            r#ref: r#ref.clone(),
            tokens,
            tool: tool.to_owned(),
            is_tool,
            index,
            is_user: message.role == Role::User,
            gap_before: pending_gap,
        });
        pending_gap = false;
    }
    (visible, summary_tokens)
}

/// Tokens per tool in first-seen order, largest first.
fn tokens_by_tool(visible: &[Visible]) -> Vec<(&str, u64)> {
    let mut by_tool: Vec<(&str, u64)> = Vec::new();
    for message in visible {
        match by_tool.iter_mut().find(|(tool, _)| *tool == message.tool) {
            Some(entry) => entry.1 += message.tokens,
            None => by_tool.push((&message.tool, message.tokens)),
        }
    }
    by_tool.sort_by_key(|(_, tokens)| Reverse(*tokens));
    by_tool
}

fn overview(visible: &[Visible], summary: u64, blocks: &[&Block], limit: usize) -> String {
    let by_tool = tokens_by_tool(visible);
    let tool: u64 = visible
        .iter()
        .filter(|message| message.is_tool)
        .map(|message| message.tokens)
        .sum();
    let text: u64 = visible
        .iter()
        .filter(|message| !message.is_tool)
        .map(|message| message.tokens)
        .sum();
    let total = summary + tool + text;
    let mut lines = vec![
        "CONTEXT BREAKDOWN".to_owned(),
        format!(
            "  {} tool ({}%) | {} text ({}%) | {} summaries ({}%)",
            format_tokens(tool),
            pct(tool, total),
            format_tokens(text),
            pct(text, total),
            format_tokens(summary),
            pct(summary, total),
        ),
    ];
    if !by_tool.is_empty() {
        let top: Vec<String> = by_tool
            .iter()
            .take(3)
            .map(|(tool, tokens)| format!("{tool} ({}%)", pct(*tokens, total)))
            .collect();
        lines.push(format!("  Top tools: {}", top.join(", ")));
    }
    lines.push(String::new());
    if blocks.is_empty() {
        lines.extend([
            "COMPRESSED BLOCKS".to_owned(),
            "  No compressed blocks.".to_owned(),
        ]);
    } else {
        let summary_total: u64 = blocks.iter().map(|block| summary_tokens(block)).sum();
        let original: u64 = blocks.iter().map(|block| block.compressed_tokens).sum();
        lines.push(format!(
            "COMPRESSED BLOCKS — {} active ({} summary, {} original)",
            blocks.len(),
            format_tokens(summary_total),
            format_tokens(original)
        ));
        if let Some(breakdown) = tier_breakdown(blocks) {
            lines.push(format!("  Tier usage: {breakdown}"));
        }
        lines.push(String::new());
        let mut sorted = blocks.to_vec();
        sorted.sort_by_key(|block| (Reverse(block.compressed_tokens), Reverse(block.created_at)));
        for block in sorted.iter().take(limit) {
            lines.push(format!(
                "  {} (T{})  {}→{}  {} msgs  \"{}\"",
                block.block_id,
                block.tier,
                format_tokens(block.compressed_tokens),
                format_tokens(summary_tokens(block)),
                block.effective_message_ids.len(),
                block.topic.as_deref().unwrap_or("(no topic)"),
            ));
        }
        if blocks.len() > limit {
            lines.push(format!(
                "  ... and {} more blocks not shown (scope:\"compressed\", limit:{} for full list)",
                blocks.len() - limit,
                blocks.len()
            ));
        }
    }
    lines.push(String::new());
    let tool_hint = by_tool
        .first()
        .map_or_else(String::new, |(tool, _)| format!(", tool:\"{tool}\""));
    lines.push(format!(
        "Tip: call again with scope:\"uncompressed\", view:\"messages\"{tool_hint} for a per-message listing"
    ));
    lines.join("\n")
}

fn message_drilldown(
    visible: &[Visible],
    tool: Option<&str>,
    sort: StatusSort,
    limit: usize,
) -> String {
    let all_tokens: u64 = visible.iter().map(|message| message.tokens).sum();
    let mut filtered: Vec<&Visible> = visible
        .iter()
        .filter(|message| tool.is_none_or(|tool| message.tool == tool))
        .collect();
    match sort {
        StatusSort::Time => filtered.sort_by_key(|message| message.index),
        StatusSort::Tool => filtered.sort_by(|a, b| {
            a.tool
                .to_lowercase()
                .cmp(&b.tool.to_lowercase())
                .then(b.tokens.cmp(&a.tokens))
        }),
        _ => filtered.sort_by_key(|message| Reverse(message.tokens)),
    }
    let total: u64 = filtered.iter().map(|message| message.tokens).sum();
    let header = match tool {
        Some(tool) => format!(
            "UNCOMPRESSED — {tool}: {} | {} msgs | {}% of visible",
            format_tokens(total),
            filtered.len(),
            pct(total, all_tokens)
        ),
        None => format!(
            "UNCOMPRESSED — {} | {} msgs",
            format_tokens(total),
            filtered.len()
        ),
    };
    let mut lines = vec![header, format!("Sorted by {}", sort.label()), String::new()];
    let shown = filtered.len().min(limit);
    for message in &filtered[..shown] {
        lines.push(format!(
            "  {} ({}) {}",
            message.r#ref,
            format_tokens(message.tokens),
            message.tool
        ));
    }
    if filtered.len() > shown {
        lines.push(String::new());
        lines.push(format!("{shown} of {} shown.", filtered.len()));
    }
    lines.join("\n")
}

fn compressed_drilldown(mut blocks: Vec<&Block>, sort: StatusSort, limit: usize) -> String {
    match sort {
        StatusSort::Time => blocks.sort_by_key(|block| block.created_at),
        StatusSort::Age => blocks.sort_by_key(|block| Reverse(block.survived_count)),
        _ => blocks
            .sort_by_key(|block| (Reverse(block.compressed_tokens), Reverse(block.created_at))),
    }
    let summary_total: u64 = blocks.iter().map(|block| summary_tokens(block)).sum();
    let original: u64 = blocks.iter().map(|block| block.compressed_tokens).sum();
    let mut lines = vec![format!(
        "COMPRESSED — {} blocks | {} original → {} summary",
        blocks.len(),
        format_tokens(original),
        format_tokens(summary_total)
    )];
    if let Some(breakdown) = tier_breakdown(&blocks) {
        lines.push(format!("Tier usage: {breakdown}"));
    }
    lines.push(String::new());
    let shown = blocks.len().min(limit);
    for block in &blocks[..shown] {
        let nested = if block.direct_block_ids.is_empty() {
            String::new()
        } else {
            format!(" nested=[{}]", block.direct_block_ids.join(","))
        };
        let generation = match block.generation {
            crate::model::Generation::Young => "young",
            crate::model::Generation::Old => "old",
        };
        lines.push(format!(
            "  {} (T{})  {}→{}  {} msgs  age={} {generation}{nested}",
            block.block_id,
            block.tier,
            format_tokens(block.compressed_tokens),
            format_tokens(summary_tokens(block)),
            block.effective_message_ids.len(),
            block.survived_count,
        ));
        lines.push(format!(
            "    \"{}\"",
            block.topic.as_deref().unwrap_or("(no topic)")
        ));
    }
    if blocks.len() > shown {
        lines.push(String::new());
        lines.push(format!("{shown} of {} shown.", blocks.len()));
    }
    lines.join("\n")
}

/// One block's summary, or a preview of every active block.
pub fn build_recap(state: &State, block_id: Option<&str>) -> String {
    let mut active: Vec<&Block> = state.active_blocks().collect();
    active.sort_by_key(|block| block_number(&block.block_id));
    if let Some(block_id) = block_id {
        let Some(block) = state.block(block_id) else {
            let ids: Vec<&str> = active.iter().map(|block| block.block_id.as_str()).collect();
            return format!(
                "Block {block_id} not found. Active blocks: {}",
                ids.join(", ")
            );
        };
        if !block.active {
            return format!("Block {block_id} is inactive (deactivated by nested compression).");
        }
        return format!(
            "[Compressed conversation section]\n{}\n\n[{block_id} | {} messages | topic: \"{}\"]",
            block.summary,
            block.effective_message_ids.len(),
            block.topic.as_deref().unwrap_or("(none)")
        );
    }
    if active.is_empty() {
        return "No active compression blocks.".to_owned();
    }
    let mut lines = vec![format!("Active compression blocks ({}):", active.len())];
    for block in &active {
        lines.push(format!(
            "\n{} | {} messages | \"{}\"",
            block.block_id,
            block.effective_message_ids.len(),
            block.topic.as_deref().unwrap_or("(none)")
        ));
        let ellipsis = if utf16_len(&block.summary) > 200 {
            "..."
        } else {
            ""
        };
        lines.push(format!("  {}{ellipsis}", clamp_prefix(&block.summary, 200)));
    }
    lines.push("\nCall with blockId to get the full summary.".to_owned());
    lines.join("\n")
}

/// The covered message ids, owned, for a projection's pre-existing
/// coverage.
pub(crate) fn coverage(state: &State) -> HashSet<String> {
    state
        .covered_message_ids()
        .into_iter()
        .map(str::to_owned)
        .collect()
}
