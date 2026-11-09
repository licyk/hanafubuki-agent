// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/truncate-tools.ts` and the emergency-truncate step of
// `kernel/src/compress.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).

use std::collections::{HashMap, HashSet};

use crate::model::{Config, Kind, Message, Role, State, TruncateLatchEntry};
use crate::nudge::NudgeDecision;
use crate::number::js_round;
use crate::prune::{is_rendered_summary, SUMMARY_HEADER};
use crate::refs::BLOCKED_REF;
use crate::render::default_count_tokens;
use crate::text::{clamp_prefix, clamp_suffix, utf16_len};

const TRUNCATION_MARKER: &str = "[truncated for context space]";

/// The latched clips kept at most; the oldest go first. A fold clears them.
const MAX_TRUNCATE_LATCH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TruncateOptions {
    /// Smaller texts are left alone.
    pub min_output_tokens: u64,
    pub keep_prefix_chars: usize,
    pub keep_suffix_chars: usize,
    /// The newest messages are never clipped.
    pub protect_recent_messages: usize,
    /// When tool results are not enough, clip user and assistant texts too
    /// (never a rendered summary).
    pub include_text_messages: bool,
}

impl Default for TruncateOptions {
    fn default() -> Self {
        Self {
            min_output_tokens: 1000,
            keep_prefix_chars: 2000,
            keep_suffix_chars: 2000,
            protect_recent_messages: 3,
            include_text_messages: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TruncateResult {
    pub messages: Vec<Message>,
    pub truncated_count: usize,
    pub saved_tokens: i64,
    /// Texts large enough to clip, whether clipped or not.
    pub candidates_found: usize,
    pub truncated_ids: Vec<String>,
}

/// The last resort at `truncate.threshold`: clips the largest tool results
/// (then, optionally, texts) outside the newest messages to their head and
/// tail around a marker, until the count is a tenth under the threshold.
/// A clip always comes out strictly shorter than its original.
pub fn truncate_large_tool_outputs(
    messages: &[Message],
    token_count: u64,
    config: &Config,
    options: &TruncateOptions,
) -> TruncateResult {
    let limit = config.model_context_limit;
    let untouched = |candidates_found| TruncateResult {
        messages: messages.to_vec(),
        truncated_count: 0,
        saved_tokens: 0,
        candidates_found,
        truncated_ids: Vec::new(),
    };
    if limit == 0 || (token_count as f64) < config.truncate.threshold * limit as f64 {
        return untouched(0);
    }
    let protected_from = messages
        .len()
        .saturating_sub(options.protect_recent_messages);
    let candidates = |predicate: &dyn Fn(&Message) -> bool| -> Vec<&Message> {
        let mut found: Vec<&Message> = messages[..protected_from]
            .iter()
            .filter(|message| predicate(message))
            .filter(|message| {
                !message.text.is_empty()
                    && !message.text.contains(TRUNCATION_MARKER)
                    && default_count_tokens(&message.text) >= options.min_output_tokens
            })
            .collect();
        found.sort_by_key(|message| std::cmp::Reverse(default_count_tokens(&message.text)));
        found
    };
    let tool_results = candidates(&|message| message.kind == Kind::ToolResult);
    let texts = if options.include_text_messages {
        candidates(&|message| {
            message.kind == Kind::Text
                && matches!(message.role, Role::User | Role::Assistant)
                && !is_rendered_summary(message)
                && !message.text.starts_with(SUMMARY_HEADER)
        })
    } else {
        Vec::new()
    };
    let candidates_found = tool_results.len() + texts.len();

    let target = config.truncate.threshold * limit as f64 * 0.9;
    let mut remaining = token_count as f64;
    let mut saved_tokens = 0;
    let mut replacements: HashMap<&str, String> = HashMap::new();
    let mut truncated_ids = Vec::new();
    for candidate in tool_results.into_iter().chain(texts) {
        if remaining <= target {
            break;
        }
        let Some(replacement) = clip(&candidate.text, options) else {
            continue;
        };
        let saved = default_count_tokens(&candidate.text) as i64
            - default_count_tokens(&replacement) as i64;
        remaining -= saved as f64;
        saved_tokens += saved;
        replacements.insert(&candidate.id, replacement);
        truncated_ids.push(candidate.id.clone());
    }
    if replacements.is_empty() {
        return untouched(candidates_found);
    }
    TruncateResult {
        messages: messages
            .iter()
            .map(|message| match replacements.get(message.id.as_str()) {
                Some(text) => Message {
                    text: text.clone(),
                    ..message.clone()
                },
                None => message.clone(),
            })
            .collect(),
        truncated_count: truncated_ids.len(),
        saved_tokens,
        candidates_found,
        truncated_ids,
    }
}

/// The head and tail of `original` around the marker, or `None` when that
/// would not come out shorter.
fn clip(original: &str, options: &TruncateOptions) -> Option<String> {
    let length = utf16_len(original);
    if length <= options.keep_prefix_chars + options.keep_suffix_chars {
        return None;
    }
    let tokens = default_count_tokens(original);
    let marker = format!("\n\n...{TRUNCATION_MARKER} — original ~{tokens} tokens]...\n\n");
    // The kept window must leave room for the marker, or the clip would grow
    // the text it is meant to shrink.
    let max_keep = length as i64 - utf16_len(&marker) as i64 - 4;
    if max_keep < 64 {
        return None;
    }
    let keep = (options.keep_prefix_chars + options.keep_suffix_chars).min(max_keep as usize);
    let prefix_keep = keep / 2;
    let replacement = format!(
        "{}{marker}{}",
        clamp_prefix(original, prefix_keep),
        clamp_suffix(original, keep - prefix_keep)
    );
    (utf16_len(&replacement) < length).then_some(replacement)
}

/// Compression and truncation can no longer bring the context under the
/// limit: the irreducible floor exceeds it.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalEscape {
    pub message: String,
    pub usage: f64,
    pub token_count: u64,
    pub model_context_limit: u64,
    /// Consecutive requests the condition has held.
    pub stuck_events: u32,
}

pub(crate) struct Emergency {
    pub(crate) messages: Vec<Message>,
    pub(crate) terminal_escape: Option<TerminalEscape>,
    pub(crate) truncation_skipped: Option<String>,
}

/// The emergency-truncate step. Clips latched earlier are replayed first,
/// below the threshold too, so the same bytes never flip between full and
/// clipped with a fluctuating count; new clips join the latch until the
/// next fold. Over the threshold with nothing worth compressing and
/// nothing to clip, a streak counts toward the terminal escape.
pub(crate) fn emergency_truncate(
    messages: Vec<Message>,
    state: &mut State,
    config: &Config,
    token_count: u64,
    nudge: &NudgeDecision,
) -> Emergency {
    let limit = config.model_context_limit;
    let usage = if limit > 0 {
        token_count as f64 / limit as f64
    } else {
        0.0
    };
    let previous_streak = state.terminal_streak;
    let ref_of = |message: &Message| {
        state
            .message_refs
            .by_raw
            .get(&message.id)
            .filter(|r#ref| *r#ref != BLOCKED_REF)
            .cloned()
    };
    // BLOCKED is shared by every protected message, not an identity.
    let latch: Vec<TruncateLatchEntry> = state
        .emergency_truncate_latch
        .iter()
        .filter(|entry| entry.r#ref != BLOCKED_REF)
        .cloned()
        .collect();
    let live_refs: HashSet<String> = messages.iter().filter_map(ref_of).collect();
    let latch_live: Vec<TruncateLatchEntry> = latch
        .iter()
        .filter(|entry| live_refs.contains(&entry.r#ref))
        .cloned()
        .collect();
    let latched: HashMap<&str, &str> = latch
        .iter()
        .map(|entry| (entry.r#ref.as_str(), entry.text.as_str()))
        .collect();
    let mut replay_saved = 0u64;
    let messages: Vec<Message> = messages
        .into_iter()
        .map(|message| {
            let clipped = ref_of(&message).and_then(|r#ref| latched.get(r#ref.as_str()).copied());
            match clipped {
                Some(clipped) if clipped != message.text => {
                    replay_saved += default_count_tokens(&message.text)
                        .saturating_sub(default_count_tokens(clipped));
                    Message {
                        text: clipped.to_owned(),
                        ..message
                    }
                }
                _ => message,
            }
        })
        .collect();

    let quiet = Emergency {
        messages,
        terminal_escape: None,
        truncation_skipped: None,
    };
    if usage < config.truncate.threshold {
        if !state.emergency_truncate_latch.is_empty() || previous_streak != 0 {
            state.terminal_streak = 0;
            state.emergency_truncate_latch = latch_live;
        }
        return quiet;
    }
    let truncated = truncate_large_tool_outputs(
        &quiet.messages,
        token_count.saturating_sub(replay_saved),
        config,
        &TruncateOptions {
            protect_recent_messages: config.preserve_recent_messages,
            include_text_messages: true,
            ..TruncateOptions::default()
        },
    );

    let min_benefit = nudge.breakdown.min_pressure_benefit;
    let max_pending = nudge.breakdown.max_pending;
    let nothing_viable = if min_benefit > 0 {
        max_pending < min_benefit
    } else {
        max_pending == 0
    };
    let stuck = nothing_viable && truncated.saved_tokens <= 0;
    let streak = if stuck { previous_streak + 1 } else { 0 };
    let escape_after = config.truncate.terminal_escape_after;
    let usage_pct = js_round(usage * 100.0) as i64;
    let truncation_skipped = (truncated.saved_tokens <= 0).then(|| {
        if truncated.candidates_found == 0 {
            format!(
                "emergency-truncate ran at {usage_pct}% usage but found no truncatable content (no oversized tool-result or text message outside the last {} messages)",
                config.preserve_recent_messages
            )
        } else {
            format!(
                "emergency-truncate found {} candidate(s) at {usage_pct}% usage but none were large enough to save tokens",
                truncated.candidates_found
            )
        }
    });
    let terminal_escape = (stuck && escape_after > 0 && streak >= escape_after).then(|| TerminalEscape {
        message: format!(
            "Usage at {usage_pct}% ({token_count}/{limit} tokens) persists with nothing compressible above the benefit floor and no truncatable content: compression cannot reduce this context below the limit. Start a new session or use native compaction."
        ),
        usage,
        token_count,
        model_context_limit: limit,
        stuck_events: streak,
    });

    let clips: Vec<TruncateLatchEntry> = truncated
        .truncated_ids
        .iter()
        .filter_map(|id| {
            let r#ref = state
                .message_refs
                .by_raw
                .get(id)
                .filter(|r#ref| *r#ref != BLOCKED_REF)?;
            let clipped = truncated
                .messages
                .iter()
                .find(|message| &message.id == id)?;
            Some(TruncateLatchEntry {
                r#ref: r#ref.clone(),
                text: clipped.text.clone(),
            })
        })
        .collect();
    let clip_refs: HashSet<&str> = clips.iter().map(|clip| clip.r#ref.as_str()).collect();
    let mut next_latch: Vec<TruncateLatchEntry> = latch_live
        .into_iter()
        .filter(|entry| !clip_refs.contains(entry.r#ref.as_str()))
        .chain(clips.iter().cloned())
        .collect();
    if next_latch.len() > MAX_TRUNCATE_LATCH {
        next_latch.drain(..next_latch.len() - MAX_TRUNCATE_LATCH);
    }
    state.terminal_streak = streak;
    state.emergency_truncate_latch = next_latch;
    Emergency {
        messages: truncated.messages,
        terminal_escape,
        truncation_skipped,
    }
}
