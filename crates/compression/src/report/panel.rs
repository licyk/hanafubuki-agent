// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/panel/panel.ts`, `cache.ts`, `format.ts` and `topic.ts` of
// acp-kernel 0.0.189 at commit `4be21b08e`. Copyright (c) 2026 ranxianglei.
// MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: no storage-mode line, no external-summary
// markers and no token-formatter override.

use crate::model::State;
use crate::nudge::{format_ranges, NudgeDecision};
use crate::number::{js_round, to_fixed};
use crate::project::viable_ranges;
use crate::render::default_count_tokens;
use crate::text::{clamp_prefix, utf16_len};

/// One request's provider-reported prompt-cache usage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheUsageSample {
    /// Prompt tokens billed without the cache.
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CacheHitSummary {
    /// The session's hit rate (0..1), weighted by billed prompt tokens.
    pub session: Option<f64>,
    /// The latest counted request's hit rate.
    pub last: Option<f64>,
    pub requests: usize,
    pub cache_read: u64,
    /// Input, cache reads and cache writes.
    pub billed_prompt: u64,
}

/// The hit rates over the requests that reported cache activity: one
/// without any (a provider with no cache reporting) is left out, not
/// counted as a miss.
pub fn cache_hit_stats(usages: &[CacheUsageSample]) -> CacheHitSummary {
    let mut summary = CacheHitSummary::default();
    for usage in usages {
        if usage.cache_read + usage.cache_write == 0 {
            continue;
        }
        let total = usage.cache_read + usage.cache_write + usage.input;
        summary.cache_read += usage.cache_read;
        summary.billed_prompt += total;
        summary.requests += 1;
        summary.last = Some(usage.cache_read as f64 / total as f64);
    }
    if summary.requests > 0 {
        summary.session = Some(summary.cache_read as f64 / summary.billed_prompt as f64);
    }
    summary
}

/// A 0..1 rate as `92.3%`.
pub fn format_hit_rate(rate: f64) -> String {
    format!("{}%", to_fixed(rate.clamp(0.0, 1.0) * 100.0, 1))
}

/// `999`, `1.2k`, `45k`, `1.2M`, `12M`.
pub fn format_compact_tokens(count: u64) -> String {
    let count_f = count as f64;
    match count {
        0..=999 => count.to_string(),
        1000..=9999 => format!("{}k", to_fixed(count_f / 1000.0, 1)),
        10_000..=999_999 => format!("{}k", js_round(count_f / 1000.0)),
        1_000_000..=9_999_999 => format!("{}M", to_fixed(count_f / 1_000_000.0, 1)),
        _ => format!("{}M", js_round(count_f / 1_000_000.0)),
    }
}

/// A short topic for a block without one: the first sentence, leading
/// quotes stripped, at most 30 characters.
pub fn topic_fallback(summary: &str) -> String {
    let first = summary.split(['.', '\n']).next().unwrap_or_default();
    let topic = first.trim().trim_start_matches(['"', '\'', '`']).trim();
    if utf16_len(topic) <= 30 {
        topic.to_owned()
    } else {
        format!("{}…", clamp_prefix(topic, 30).trim_end())
    }
}

/// What the status panel shows.
pub struct StatusPanelInput<'a> {
    /// A header line naming the host, if any.
    pub version: Option<&'a str>,
    /// The host's session accounting: the number its footer shows.
    pub token_count: u64,
    pub system_prompt_tokens: u64,
    pub state: &'a State,
    /// This request's decision.
    pub nudge: Option<&'a NudgeDecision>,
    pub model_context_limit: u64,
    /// The estimate of the whole unpruned history, on the same scale as the
    /// breakdown; without it the session-only line is left out.
    pub unpruned_tokens: Option<u64>,
    /// Per-request cache usage; `None` hides the cache line.
    pub cache_usages: Option<&'a [CacheUsageSample]>,
}

/// `value`'s share of `total` as a bar of `width` cells, `█` filled and `░`
/// empty (the panel's are 20); empty when `total` is 0.
pub fn bar(value: u64, total: u64, width: usize) -> String {
    if total == 0 {
        return String::new();
    }
    let filled =
        (js_round(value as f64 / total as f64 * width as f64).max(0.0) as usize).min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// The status panel. Its three token numbers each say their scale and are
/// never combined: the host's accounting (compressed originals included),
/// the view sent to the model, and what pruning leaves out of it.
pub fn build_status_panel(input: &StatusPanelInput<'_>) -> String {
    let fmt = format_compact_tokens;
    let limit = input.model_context_limit;
    let breakdown = input
        .nudge
        .and_then(|nudge| nudge.context_breakdown.as_ref());
    let classified = breakdown.map_or(0, |breakdown| {
        breakdown.system + breakdown.tool + breakdown.summaries + breakdown.code + breakdown.text
    });
    let sent = classified + input.system_prompt_tokens;
    let session_only = input
        .unpruned_tokens
        .map_or(0, |unpruned| unpruned.saturating_sub(sent));
    let percent_of_limit = |tokens: u64| {
        if limit > 0 {
            js_round(tokens as f64 / limit as f64 * 100.0) as u64
        } else {
            0
        }
    };
    let state = input.state;

    let mut lines = vec![
        "╭─────────────────────────────────────────────╮".to_owned(),
        "│           ACP Context Analysis              │".to_owned(),
        "╰─────────────────────────────────────────────╯".to_owned(),
    ];
    lines.extend(input.version.map(str::to_owned));
    lines.push(String::new());
    lines.push(format!(
        "Context (session accounting, host footer scale): {}% ({} / {}) — includes compressed originals; shrinks slower than the sent view",
        percent_of_limit(input.token_count),
        fmt(input.token_count),
        fmt(limit)
    ));

    if let (Some(_), Some(breakdown)) = (input.nudge, breakdown) {
        if breakdown.growth > 0 && input.token_count > 0 {
            lines.push(format!(
                "Growth: +{} since last nudge",
                fmt(breakdown.growth as u64)
            ));
        }
        lines.push(String::new());
        let of_limit = if limit > 0 {
            format!(" ({}% of limit)", percent_of_limit(sent))
        } else {
            String::new()
        };
        lines.push(format!(
            "Sent to LLM (after compression, est.): {}{of_limit}",
            fmt(sent)
        ));
        if input.unpruned_tokens.is_some() && session_only > 0 {
            lines.push(format!(
                "Session-only (compressed originals, est.): {} — pruned from every request; the footer/nudge still count them",
                fmt(session_only)
            ));
        }
        lines.push(String::new());
        lines.push("Token Breakdown (sent view):".to_owned());
        for (label, value) in [
            ("Tool", breakdown.tool),
            ("SysPrompt", input.system_prompt_tokens),
            ("Text", breakdown.text),
            ("Code", breakdown.code),
            ("Summaries", breakdown.summaries),
        ] {
            if value == 0 {
                continue;
            }
            let pct = if sent > 0 {
                js_round(value as f64 / sent as f64 * 100.0) as u64
            } else {
                0
            };
            lines.push(format!(
                "  {label:<10} {} {pct:>3}%  {}",
                bar(value, sent, 20),
                fmt(value)
            ));
        }
    }

    if let Some(usages) = input.cache_usages {
        let cache = cache_hit_stats(usages);
        if let (Some(session), Some(last)) = (cache.session, cache.last) {
            lines.push(String::new());
            lines.push(format!(
                "Prompt cache (provider-reported): {} last · {} session avg — {} of {} billed prompt tokens served from cache ({} req)",
                format_hit_rate(last),
                format_hit_rate(session),
                fmt(cache.cache_read),
                fmt(cache.billed_prompt),
                cache.requests
            ));
        }
    }
    lines.push(String::new());
    lines.extend(nudge_lines(input.nudge));
    lines.extend(block_lines(state));
    lines.push(String::new());
    lines.push(
        "Tag visibility: tags injected to LLM only (deep copy), not persisted in session, not shown in terminal."
            .to_owned(),
    );
    lines.join("\n")
}

fn nudge_lines(nudge: Option<&NudgeDecision>) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(nudge) = nudge {
        lines.push(if nudge.should_inject {
            let tier = nudge
                .tier
                .map_or_else(String::new, |tier| format!(" [T{tier} distillation]"));
            format!("Nudge: ACTIVE{tier} — {}", nudge.reason)
        } else {
            format!("Nudge: idle — {}", nudge.reason)
        });
    }
    let ranges = viable_ranges(
        nudge.map_or_else(Vec::new, |nudge| nudge.compressible_ranges.clone()),
        |range| range.tokens,
    );
    let protected = nudge.map_or(&[][..], |nudge| &nudge.protected_ranges);
    if !ranges.is_empty() || !protected.is_empty() {
        lines.push(String::new());
        lines.push(format_ranges(&ranges, protected));
    }
    lines
}

fn block_lines(state: &State) -> Vec<String> {
    let fmt = format_compact_tokens;
    let active: Vec<_> = state.active_blocks().collect();
    let total = state.blocks.len();
    let compressed = fmt(state.stats.tokens_compressed);
    if active.is_empty() {
        return vec![
            String::new(),
            if total > 0 {
                format!(
                    "Blocks: 0 active / {total} total ({compressed} tokens compressed, cumulative)"
                )
            } else {
                "Blocks: none (nothing compressed yet)".to_owned()
            },
        ];
    }
    let mut lines = vec![
        String::new(),
        format!(
            "Blocks: {} active / {total} total ({compressed} tokens compressed, cumulative)",
            active.len()
        ),
    ];
    for block in active {
        let topic = block
            .topic
            .clone()
            .filter(|topic| !topic.is_empty())
            .unwrap_or_else(|| topic_fallback(&block.summary));
        let summary = default_count_tokens(&block.summary);
        let original = if block.compressed_tokens > 0 {
            block.compressed_tokens
        } else {
            summary
        };
        lines.push(format!(
            "  [{}] T{} {}→{}: {topic}",
            block.block_id,
            block.tier,
            fmt(original),
            fmt(summary)
        ));
    }
    lines
}
