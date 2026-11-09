// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/nudge-text.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: no section overrides (packs), and the lane
// is only whether it speaks Anthropic Messages.

use super::{ContextBreakdown, NudgeDecision};
use crate::boundaries::BlockSpan;
use crate::model::Block;
use crate::number::format_k;
use crate::prompts::Prompts;
use crate::recommend::{CompressibleRange, ProtectedRange};
use crate::refs::ordered_ref_pair;
use crate::text::utf16_len;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NudgeVoice {
    Gentle,
    Emergency,
}

/// The nudge as the host appends it, as an ephemeral trailing user message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedNudge {
    pub voice: NudgeVoice,
    pub text: String,
}

/// One call carrying every range: each separate call rewrites the cached
/// prefix from its fold onward and bills the rest of the history again.
/// Static, so it never reads as a to-do list.
pub const ONE_CALL_HINT: &str = r##"ONE call, ONE string — fold every range you keep into a single compress call: one JSON-escaped string holding one block per range ('m00150–m00220 topic' header line, then the summary). Ranges you still need can wait — they reappear in later nudges; never split the batch across separate calls."##;

/// On Anthropic Messages lanes the model tends to write a compaction
/// envelope as its reply instead of calling the tool, which ends the turn.
const ANTHROPIC_LANE_NOTE: &str = "Write the summary ONLY as the `compress` tool's `content` argument. Never emit `<analysis>` or `<summary>` tags in your reply text — they are not parsed, and a reply without a tool call ends the turn.";

const GENTLE_CLOSING: &str = r##"💡 If you compress, fold the ranges you keep in ONE call — ONE plain string holding every range, each block starting with its 'mNNNNN–mNNNNN topic' header line (a valid JSON string: \" and \n). Ranges the task still needs can wait — they reappear in later nudges."##;

const T2_GUIDANCE: &str = "Your tier-1 compression summaries have accumulated. Distill them into a single denser tier-2 summary. Use block IDs as boundaries (startId and endId as bN). Any raw (uncompressed) messages sitting between the boundary blocks are absorbed into the tier-2 block as well — apply HOW TO COMPRESS to those raw messages and the TIER 2 distillation rules to the existing summaries, so the whole span is covered and nothing is lost.";

const T3_GUIDANCE: &str = "Your tier-2 compression summaries have accumulated. Condense them further into a tier-3 ultra-condensed summary. Use block IDs as boundaries (startId and endId as bN). Any raw (uncompressed) messages sitting between the boundary blocks are absorbed into the tier-3 block as well — apply HOW TO COMPRESS to those raw messages and the TIER 3 condensation rules to the existing summaries, so the whole span is covered and nothing is lost.";

const BLOCK_MAP_MAX_SHOWN: usize = 8;

/// The nudge's text: a tier-2 or tier-3 distillation request, the
/// emergency voice (over the pressure band) or the gentle efficiency note,
/// each with the context breakdown and the rules; the message-range voices
/// list the ranges and the active blocks.
pub fn render_nudge_text(
    decision: &NudgeDecision,
    prompts: &Prompts,
    anthropic_lane: bool,
) -> RenderedNudge {
    let emergency = decision.breakdown.emergency_override || decision.breakdown.over_limit;
    let voice = if emergency {
        NudgeVoice::Emergency
    } else {
        NudgeVoice::Gentle
    };
    let mut parts = match decision.tier {
        Some(tier) if tier >= 2 => tier_parts(decision, prompts, tier, emergency),
        _ => range_parts(decision, prompts, emergency),
    };
    if anthropic_lane {
        parts.extend([String::new(), ANTHROPIC_LANE_NOTE.to_owned()]);
    }
    let first = parts
        .iter()
        .position(|part| !part.is_empty())
        .unwrap_or(parts.len());
    RenderedNudge {
        voice,
        text: parts[first..].join("\n"),
    }
}

fn efficiency_note(prompts: &Prompts) -> String {
    format!(
        "This is an efficiency nudge to compress early and keep context lean — not an overflow warning. A separate, stronger alert will appear if the context is actually full.\n\n{}",
        prompts.compress_philosophy
    )
}

fn tier_parts(
    decision: &NudgeDecision,
    prompts: &Prompts,
    tier: u8,
    emergency: bool,
) -> Vec<String> {
    let distil = tier == 2;
    let targets = &decision.tier_target_blocks;
    let start = targets
        .first()
        .map_or("b1", |block| block.block_id.as_str());
    let end = targets.last().map_or("b5", |block| block.block_id.as_str());
    let kind = if distil {
        "DISTILLATION"
    } else {
        "CONDENSATION"
    };
    let trigger = if emergency {
        format!("[EMERGENCY — TIER {tier} {kind}] Context limit reached — distill NOW into a denser summary to reclaim tokens.")
    } else {
        format!("[TIER {tier} {kind} TRIGGER]")
    };
    vec![
        efficiency_note(prompts),
        String::new(),
        format_breakdown(decision.context_breakdown.as_ref()),
        String::new(),
        trigger,
        (if distil { T2_GUIDANCE } else { T3_GUIDANCE }).to_owned(),
        format_tier_target_blocks(targets),
        format!("Example: compress({{ content: \"{start}–{end} optional topic\\nsummary markdown...\" }})"),
        String::new(),
        prompts.how_to_compress_rules.clone(),
        String::new(),
        if distil {
            prompts.tier2_distill_rules.clone()
        } else {
            prompts.tier3_condense_rules.clone()
        },
    ]
}

fn range_parts(decision: &NudgeDecision, prompts: &Prompts, emergency: bool) -> Vec<String> {
    let breakdown = format_breakdown(decision.context_breakdown.as_ref());
    let ranges = format_ranges(&decision.compressible_ranges, &decision.protected_ranges);
    let block_map = format_block_map(&decision.active_block_spans);
    let mut parts = if emergency {
        vec![
            format!(
                "⚠️ Context limit reached — compress now. Prioritize consumed tool outputs.\n\n{}",
                prompts.compress_philosophy
            ),
            String::new(),
            breakdown,
            String::new(),
            prompts.how_to_compress_rules.clone(),
            String::new(),
            "Only use IDs from visible messages above. Compress older work first.".to_owned(),
            String::new(),
            ranges,
        ]
    } else {
        vec![
            efficiency_note(prompts),
            String::new(),
            breakdown,
            String::new(),
            prompts.how_to_compress_rules.clone(),
            String::new(),
            ranges,
        ]
    };
    if !block_map.is_empty() {
        parts.extend([String::new(), block_map]);
    }
    parts.push(String::new());
    parts.push(
        if emergency {
            ONE_CALL_HINT
        } else {
            GENTLE_CLOSING
        }
        .to_owned(),
    );
    parts
}

fn format_breakdown(breakdown: Option<&ContextBreakdown>) -> String {
    let Some(breakdown) = breakdown else {
        return String::new();
    };
    let parts: Vec<String> = [
        (breakdown.system, "system"),
        (breakdown.tool, "tool"),
        (breakdown.summaries, "summaries"),
        (breakdown.code, "code"),
        (breakdown.text, "text"),
    ]
    .into_iter()
    .filter(|(tokens, _)| *tokens > 0)
    .map(|(tokens, label)| format!("{} {label}", format_k(tokens as f64)))
    .collect();
    let growth = if breakdown.growth > 0 {
        format!("\n+{} since last nudge", format_k(breakdown.growth as f64))
    } else {
        String::new()
    };
    format!("Context breakdown: {}{growth}", parts.join(" | "))
}

fn format_tier_target_blocks(blocks: &[Block]) -> String {
    let Some(first) = blocks.first() else {
        return "Target blocks: (none — no tier blocks found)".to_owned();
    };
    let lines: Vec<String> = blocks
        .iter()
        .map(|block| {
            // Characters over four, not the token estimate.
            let summary_tokens = utf16_len(&block.summary).div_ceil(4);
            let topic = block
                .topic
                .as_ref()
                .map_or_else(String::new, |topic| format!("  \"{topic}\""));
            format!(
                "  {}  {} msgs  {}→{}{topic}",
                block.block_id,
                block.effective_message_ids.len(),
                format_k(block.compressed_tokens as f64),
                format_k(summary_tokens as f64),
            )
        })
        .collect();
    let tier = if first.tier == 1 { "tier-1" } else { "tier-2" };
    format!(
        "Target {tier} blocks to distill ({}):\n{}",
        blocks.len(),
        lines.join("\n")
    )
}

fn format_block_map(spans: &[BlockSpan]) -> String {
    if spans.is_empty() {
        return String::new();
    }
    let hidden = spans.len().saturating_sub(BLOCK_MAP_MAX_SHOWN);
    let items: Vec<String> = spans[hidden..]
        .iter()
        .map(|span| {
            let tier = if span.tier > 1 {
                format!(" t{}", span.tier)
            } else {
                String::new()
            };
            format!(
                "{}={}–{}{tier}",
                span.block_id, span.start_ref, span.end_ref
            )
        })
        .collect();
    let prefix = if hidden > 0 {
        format!("…+{hidden} older · ")
    } else {
        String::new()
    };
    format!(
        "Active blocks ({}): {prefix}{}",
        spans.len(),
        items.join(" · ")
    )
}

/// A compressible or protected range in the merged listing.
struct Entry {
    start_ref: String,
    end_ref: String,
    start_number: u64,
    start_position: u64,
    end_position: u64,
    count: usize,
    tokens: u64,
    user_msgs: usize,
    compressible_tokens: u64,
    protected_tokens: u64,
    protected_tools: Vec<String>,
    tool_pct: u32,
    text_pct: u32,
    no_merge_before: bool,
}

/// The compressible and protected ranges as one oldest-first list, ordered
/// and merged by position (a gap of at most one slot), never by ref number,
/// since refs need not follow the history's order.
pub fn format_ranges(compressible: &[CompressibleRange], protected: &[ProtectedRange]) -> String {
    if compressible.is_empty() && protected.is_empty() {
        return "[No specific ranges detected — compress any consumed content.]".to_owned();
    }
    let mut entries: Vec<Entry> = compressible
        .iter()
        .map(|range| Entry {
            tool_pct: range.tool_pct,
            text_pct: range.text_pct,
            user_msgs: range.user_msgs,
            compressible_tokens: range.tokens,
            ..entry(
                &range.start_ref,
                &range.end_ref,
                (range.start_index, range.end_index),
                range.count,
                range.tokens,
                range.no_merge_before,
            )
        })
        .chain(protected.iter().map(|range| Entry {
            protected_tokens: range.tokens,
            protected_tools: range.tools.clone(),
            ..entry(
                &range.start_ref,
                &range.end_ref,
                (range.start_index, range.end_index),
                range.count,
                range.tokens,
                range.no_merge_before,
            )
        }))
        .collect();
    entries.sort_by_key(|entry| (entry.start_position, entry.start_number));

    let mut merged: Vec<Entry> = Vec::new();
    for entry in entries {
        match merged.last_mut() {
            Some(last)
                if !entry.no_merge_before && entry.start_position <= last.end_position + 1 =>
            {
                last.end_ref = entry.end_ref;
                last.end_position = last.end_position.max(entry.end_position);
                last.count += entry.count;
                last.tokens += entry.tokens;
                last.user_msgs += entry.user_msgs;
                last.compressible_tokens += entry.compressible_tokens;
                last.protected_tokens += entry.protected_tokens;
                for tool in entry.protected_tools {
                    if !last.protected_tools.contains(&tool) {
                        last.protected_tools.push(tool);
                    }
                }
            }
            _ => merged.push(entry),
        }
    }
    let lines: Vec<String> = merged.iter().map(range_line).collect();
    format!(
        "Compressible ranges ({}, oldest first):\n{}",
        merged.len(),
        lines.join("\n")
    )
}

fn entry(
    start_ref: &str,
    end_ref: &str,
    (start_index, end_index): (Option<usize>, Option<usize>),
    count: usize,
    tokens: u64,
    no_merge_before: bool,
) -> Entry {
    Entry {
        start_ref: start_ref.to_owned(),
        end_ref: end_ref.to_owned(),
        start_number: first_number(start_ref),
        start_position: start_index.map_or_else(|| first_number(start_ref), |index| index as u64),
        end_position: end_index.map_or_else(|| first_number(end_ref), |index| index as u64),
        count,
        tokens,
        user_msgs: 0,
        compressible_tokens: 0,
        protected_tokens: 0,
        protected_tools: Vec::new(),
        tool_pct: 0,
        text_pct: 0,
        no_merge_before,
    }
}

fn range_line(entry: &Entry) -> String {
    // Endpoints in ascending order: the direction of a pair is notational.
    let (low, high) = ordered_ref_pair(&entry.start_ref, &entry.end_ref);
    let users = match entry.user_msgs {
        0 => String::new(),
        1 => " · 1 user msg".to_owned(),
        count => format!(" · {count} user msgs"),
    };
    let head = format!(
        "  {low}–{high}  {} msgs  {}",
        entry.count,
        format_k(entry.tokens as f64)
    );
    let tools = entry.protected_tools.join(", ");
    if entry.protected_tokens > 0 && entry.compressible_tokens == 0 {
        return format!("{head} [PROTECTED: {tools} — not compressible]");
    }
    if entry.protected_tokens > 0 {
        return format!(
            "{head} [{} compressible | {} protected: {tools}]{users}",
            format_k(entry.compressible_tokens as f64),
            format_k(entry.protected_tokens as f64),
        );
    }
    format!(
        "{head} [tool {}% | text {}%]{users}",
        entry.tool_pct, entry.text_pct
    )
}

/// The first run of digits in a ref, or zero.
fn first_number(r#ref: &str) -> u64 {
    let digits: String = r#ref
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().unwrap_or(0)
}
