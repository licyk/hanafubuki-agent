// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/recommend.ts` and the recommend step of `kernel/src/compress.ts`
// of acp-kernel 0.0.189 at commit `4be21b08e`. Copyright (c) 2026
// ranxianglei. MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: there are no host checkpoint carriers and
// no unrecoverable media to leave gaps for, and no host-set `dangerous`
// flag.

mod merge;
mod screen;

use std::collections::HashSet;

pub use merge::merge_ranges_to_threshold;
pub use screen::screen_recommended_units;

use crate::model::{Config, Message, Role, State};
use crate::number::js_round;
use crate::project::{compute_integrity_withdrawals, segment_groups, SegmentItem};
use crate::protect::{
    collect_failed_compress_call_ids, collect_latest_protected, compute_protected_refs,
    is_message_latest_protected,
};
use crate::protect::{collect_protected_tool_call_ids, is_message_protected_with_pairing};
use crate::prune::SUMMARY_HEADER;
use crate::refs::{ref_to_index, BLOCKED_REF};
use crate::render::count_message_tokens;
use crate::text::utf16_len;

/// A run of messages that may fold, as the nudge and `acp_status` offer it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompressibleRange {
    pub start_ref: String,
    pub end_ref: String,
    pub count: usize,
    pub tokens: u64,
    /// UTF-16 code units of the messages' text: the unit the apply step's
    /// minimum range counts in. `None` on a hand-built range, which counts
    /// as four characters a token.
    pub chars: Option<usize>,
    /// Shares of tool and text messages, in whole percent.
    pub tool_pct: u32,
    pub text_pct: u32,
    pub user_msgs: usize,
    /// Positions of the first and last message in the history: ranges merge
    /// by position, never by ref number.
    pub start_index: Option<usize>,
    pub end_index: Option<usize>,
    /// The range opens right after a dead ref, which leaves no gap in the
    /// positions, so nothing may merge across it.
    pub no_merge_before: bool,
}

impl CompressibleRange {
    /// Its size as the apply step's minimum range counts it.
    pub(crate) fn effective_chars(&self) -> usize {
        self.chars.unwrap_or(self.tokens as usize * 4)
    }
}

/// A run of protected messages, shown beside the compressible ones.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProtectedRange {
    pub start_ref: String,
    pub end_ref: String,
    pub count: usize,
    pub tokens: u64,
    pub tools: Vec<String>,
    pub start_index: Option<usize>,
    pub end_index: Option<usize>,
    pub no_merge_before: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextRanges {
    pub compressible: Vec<CompressibleRange>,
    pub protected: Vec<ProtectedRange>,
}

/// What may fold this request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recommendation {
    pub context_ranges: ContextRanges,
    /// The compressible ranges merged to the minimum range and screened by
    /// the apply step's own projection: every one of them folds.
    pub recommended_ranges: Vec<CompressibleRange>,
    pub nothing_to_compress: bool,
}

/// The recommend step: ranges are built on the view, then merged and
/// screened on `original`, the full history the apply step runs on, each
/// with the recent zone of its own array.
pub fn recommend(
    view: &[Message],
    original: &[Message],
    state: &State,
    config: &Config,
) -> Recommendation {
    let zone = compute_protected_refs(view, state, config);
    let context_ranges = build_compressible_ranges(view, state, config, Some(&zone), original);
    let nothing_to_compress = context_ranges.compressible.is_empty();
    let merged = merge_ranges_to_threshold(
        context_ranges.compressible.clone(),
        config.compress.min_compress_range,
    );
    let original_zone = compute_protected_refs(original, state, config);
    let recommended_ranges =
        screen_recommended_units(merged, original, state, config, &original_zone);
    Recommendation {
        context_ranges,
        recommended_ranges,
        nothing_to_compress,
    }
}

#[derive(Clone)]
struct CompressibleInfo {
    id: String,
    r#ref: String,
    gap_before: bool,
    tokens: u64,
    chars: usize,
    is_tool: bool,
    is_user: bool,
    index: usize,
}

impl SegmentItem for CompressibleInfo {
    fn gap_before(&self) -> bool {
        self.gap_before
    }

    fn is_user(&self) -> bool {
        self.is_user
    }
}

struct ProtectedInfo {
    r#ref: String,
    gap_before: bool,
    tokens: u64,
    tool: Option<String>,
    index: usize,
}

/// Groups the messages with numbered refs into compressible runs and
/// protected runs. Covered messages, rendered summaries, the recent zone
/// and protected messages separate runs; runs follow array positions, never
/// ref arithmetic, and split at a user message once three long. A message
/// the turn-integrity gate would withdraw (judged on `integrity_source`,
/// the array the apply step runs on) is left out as a gap, so every range
/// can fold.
pub fn build_compressible_ranges(
    messages: &[Message],
    state: &State,
    config: &Config,
    protected_zone: Option<&HashSet<String>>,
    integrity_source: &[Message],
) -> ContextRanges {
    let covered = state.covered_message_ids();
    let (compressible, protected) = classify(messages, state, config, protected_zone, &covered);

    let candidate_ids: HashSet<String> = compressible.iter().map(|info| info.id.clone()).collect();
    let covered_owned: HashSet<String> = covered.iter().map(|id| (*id).to_owned()).collect();
    let withdrawn: HashSet<String> =
        compute_integrity_withdrawals(integrity_source, &candidate_ids, Some(&covered_owned))
            .withdrawn
            .into_iter()
            .collect();
    let compressible = mark_withdrawn_gaps(compressible, &withdrawn);

    let mut compressible: Vec<CompressibleRange> = segment_groups(&compressible)
        .iter()
        .map(|group| aggregate(group))
        .collect();
    let mut protected = group_protected(protected);
    mark_no_merge_across_dead_holes(&mut compressible, &mut protected, &dead_ref_numbers(state));
    compressible.retain(|range| range.tokens > 0);
    ContextRanges {
        compressible,
        protected,
    }
}

/// One pass over the messages: each numbered message is compressible,
/// protected or a gap.
fn classify(
    messages: &[Message],
    state: &State,
    config: &Config,
    protected_zone: Option<&HashSet<String>>,
    covered: &HashSet<&str>,
) -> (Vec<CompressibleInfo>, Vec<ProtectedInfo>) {
    let mut protected_call_ids = collect_protected_tool_call_ids(messages, config);
    let failed_compress_call_ids = collect_failed_compress_call_ids(messages);
    let latest = collect_latest_protected(messages, config);
    protected_call_ids.extend(latest.call_ids.iter().cloned());
    let dead_numbers = dead_ref_numbers(state);

    let mut compressible = Vec::new();
    let mut protected = Vec::new();
    let mut skip_since_compressible = false;
    let mut skip_since_protected = false;
    let mut last_compressible_number: Option<u32> = None;
    for (index, message) in messages.iter().enumerate() {
        let Some(r#ref) = state
            .message_refs
            .by_raw
            .get(&message.id)
            .filter(|r#ref| *r#ref != BLOCKED_REF)
        else {
            continue;
        };
        if message.text.starts_with(SUMMARY_HEADER) || covered.contains(message.id.as_str()) {
            skip_since_compressible = true;
            skip_since_protected = true;
            continue;
        }
        if is_message_protected_with_pairing(
            message,
            config,
            &protected_call_ids,
            &failed_compress_call_ids,
        ) || is_message_latest_protected(message, &latest)
        {
            protected.push(ProtectedInfo {
                r#ref: r#ref.clone(),
                gap_before: skip_since_protected,
                tokens: count_message_tokens(message),
                tool: message.tool_name.clone(),
                index,
            });
            skip_since_protected = false;
            skip_since_compressible = true;
            continue;
        }
        if protected_zone.is_some_and(|zone| zone.contains(r#ref)) {
            skip_since_compressible = true;
            skip_since_protected = true;
            continue;
        }
        // A dropped message leaves no array slot, so its dead ref breaks the
        // run here.
        let number = ref_to_index(r#ref);
        if let (Some(current), Some(last)) = (number, last_compressible_number) {
            if has_dead_ref_between(&dead_numbers, current.min(last), current.max(last)) {
                skip_since_compressible = true;
            }
        }
        compressible.push(CompressibleInfo {
            id: message.id.clone(),
            r#ref: r#ref.clone(),
            gap_before: skip_since_compressible,
            tokens: count_message_tokens(message),
            chars: utf16_len(&message.text),
            is_tool: message.is_tool(),
            is_user: message.role == Role::User,
            index,
        });
        if number.is_some() {
            last_compressible_number = number;
        }
        skip_since_compressible = false;
        skip_since_protected = true;
    }
    (compressible, protected)
}

/// Drops the withdrawn messages, each leaving a gap before the next kept
/// one.
fn mark_withdrawn_gaps(
    compressible: Vec<CompressibleInfo>,
    withdrawn: &HashSet<String>,
) -> Vec<CompressibleInfo> {
    if withdrawn.is_empty() {
        return compressible;
    }
    let mut gap_pending = false;
    let mut kept = Vec::with_capacity(compressible.len());
    for mut info in compressible {
        if withdrawn.contains(&info.id) {
            gap_pending = true;
            continue;
        }
        if gap_pending {
            info.gap_before = true;
        }
        kept.push(info);
        gap_pending = false;
    }
    kept
}

fn aggregate(group: &[CompressibleInfo]) -> CompressibleRange {
    let first = &group[0];
    let mut range = CompressibleRange {
        start_ref: first.r#ref.clone(),
        end_ref: first.r#ref.clone(),
        count: 1,
        tokens: first.tokens,
        chars: Some(first.chars),
        tool_pct: if first.is_tool { 100 } else { 0 },
        text_pct: if first.is_tool { 0 } else { 100 },
        user_msgs: usize::from(first.is_user),
        start_index: Some(first.index),
        end_index: Some(first.index),
        no_merge_before: false,
    };
    for info in &group[1..] {
        range.end_ref = info.r#ref.clone();
        range.end_index = Some(info.index);
        range.count += 1;
        range.tokens += info.tokens;
        range.chars = Some(range.effective_chars() + info.chars);
        range.user_msgs += usize::from(info.is_user);
        let tool_share = f64::from(range.tool_pct) * (range.count - 1) as f64
            + if info.is_tool { 100.0 } else { 0.0 };
        range.tool_pct = js_round(tool_share / range.count as f64) as u32;
        range.text_pct = 100 - range.tool_pct;
    }
    range
}

fn group_protected(infos: Vec<ProtectedInfo>) -> Vec<ProtectedRange> {
    let mut ranges: Vec<ProtectedRange> = Vec::new();
    let mut current: Option<ProtectedRange> = None;
    for info in infos {
        if info.gap_before {
            ranges.extend(current.take());
        }
        match &mut current {
            None => {
                current = Some(ProtectedRange {
                    start_ref: info.r#ref.clone(),
                    end_ref: info.r#ref,
                    count: 1,
                    tokens: info.tokens,
                    tools: info.tool.into_iter().collect(),
                    start_index: Some(info.index),
                    end_index: Some(info.index),
                    no_merge_before: false,
                });
            }
            Some(range) => {
                range.end_ref = info.r#ref;
                range.end_index = Some(info.index);
                range.count += 1;
                range.tokens += info.tokens;
                if let Some(tool) = info.tool {
                    if !range.tools.contains(&tool) {
                        range.tools.push(tool);
                    }
                }
            }
        }
    }
    ranges.extend(current);
    ranges
}

fn dead_ref_numbers(state: &State) -> HashSet<u32> {
    state
        .dead_refs
        .iter()
        .filter_map(|r#ref| ref_to_index(r#ref))
        .collect()
}

/// A dead ref strictly between two live ref numbers.
fn has_dead_ref_between(dead: &HashSet<u32>, low: u32, high: u32) -> bool {
    high - low > 1 && dead.iter().any(|number| *number > low && *number < high)
}

/// Flags every range, compressible or protected, that opens right after a
/// dead-ref hole, so the position-based merges cannot bridge it.
fn mark_no_merge_across_dead_holes(
    compressible: &mut [CompressibleRange],
    protected: &mut [ProtectedRange],
    dead: &HashSet<u32>,
) {
    if dead.is_empty() || compressible.len() + protected.len() < 2 {
        return;
    }
    // (start position, start ref, end ref, is protected, index in its list)
    let mut order: Vec<(usize, String, String, bool, usize)> = compressible
        .iter()
        .enumerate()
        .map(|(index, range)| {
            let position = range
                .start_index
                .or_else(|| ref_to_index(&range.start_ref).map(|number| number as usize))
                .unwrap_or(0);
            let refs = (range.start_ref.clone(), range.end_ref.clone());
            (position, refs.0, refs.1, false, index)
        })
        .chain(protected.iter().enumerate().map(|(index, range)| {
            let position = range
                .start_index
                .or_else(|| ref_to_index(&range.start_ref).map(|number| number as usize))
                .unwrap_or(0);
            let refs = (range.start_ref.clone(), range.end_ref.clone());
            (position, refs.0, refs.1, true, index)
        }))
        .collect();
    order.sort_by_key(|entry| entry.0);
    for pair in order.windows(2) {
        let (Some(previous_end), Some(current_start)) =
            (ref_to_index(&pair[0].2), ref_to_index(&pair[1].1))
        else {
            continue;
        };
        let (low, high) = (
            previous_end.min(current_start),
            previous_end.max(current_start),
        );
        if has_dead_ref_between(dead, low, high) {
            let (_, _, _, is_protected, index) = pair[1];
            if is_protected {
                protected[index].no_merge_before = true;
            } else {
                compressible[index].no_merge_before = true;
            }
        }
    }
}
