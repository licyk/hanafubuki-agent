// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/crush.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE). Its log strategy is the kernel's port of headroom's
// LogCompressor (headroomlabs-ai/headroom, Apache-2.0), and its JSON folding
// adapts headroom's SmartCrusher concept.
// Modified for hanafubuki-agent: the three built-in strategies are the only
// ones (no plugin registry, so the plugin-contract guards have nothing to
// guard); tokens are the crate's estimate.

mod code;
mod json;
mod log;

use crate::absorb::ABSORB_PROMPT_MARKER;
use crate::model::{Config, CrushConfig, Message};
use crate::protect::{collect_protected_tool_call_ids, match_tool_pattern};
use crate::render::default_count_tokens;
use crate::text::utf16_len;

pub use code::crush_code;
pub use json::crush_json;
pub use log::crush_log;

/// Larger inputs are never crushed.
const MAX_INPUT_CHARS: usize = 2_000_000;

/// What a tool result's text looks like, which decides the one strategy
/// that may see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrushKind {
    Json,
    Code,
    Log,
}

/// The built-in strategies, in the order they are tried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrushStrategy {
    /// Lossless: constant fields hoisted, identical runs counted
    /// (`__acp_crush` envelopes).
    JsonFold,
    /// Lossy: full-line comments, docstrings and blank runs elided.
    CodeTrim,
    /// Lossy: errors, warnings, traces and summaries kept, noise dropped.
    LogSelect,
}

pub const CRUSH_STRATEGIES: [CrushStrategy; 3] = [
    CrushStrategy::JsonFold,
    CrushStrategy::CodeTrim,
    CrushStrategy::LogSelect,
];

impl CrushStrategy {
    pub fn id(self) -> &'static str {
        match self {
            Self::JsonFold => "json-fold",
            Self::CodeTrim => "code-trim",
            Self::LogSelect => "log-select",
        }
    }

    pub fn kind(self) -> CrushKind {
        match self {
            Self::JsonFold => CrushKind::Json,
            Self::CodeTrim => CrushKind::Code,
            Self::LogSelect => CrushKind::Log,
        }
    }

    pub fn lossy(self) -> bool {
        self != Self::JsonFold
    }

    /// The strategy's output, or `None` to leave the text as it is.
    pub fn run(self, text: &str) -> Option<String> {
        match self {
            Self::JsonFold => crush_json(text),
            Self::CodeTrim => crush_code(text),
            Self::LogSelect => crush_log(text),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CrushOutput {
    pub text: String,
    pub strategy: CrushStrategy,
}

/// JSON when it is bracketed, code when a language is detected, else a log.
pub fn classify_crush_text(text: &str) -> CrushKind {
    let trimmed = text.trim();
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        CrushKind::Json
    } else if code::detect_language(text).is_some() {
        CrushKind::Code
    } else {
        CrushKind::Log
    }
}

/// Crushes `text` with the first strategy of its kind, enabled and not
/// excluding `tool_name`, whose output saves at least `min_reduction` of
/// the tokens; a lossy log output must keep every error line. `None`
/// leaves the text as it is.
pub fn crush_text(
    text: &str,
    config: &CrushConfig,
    tool_name: Option<&str>,
) -> Option<CrushOutput> {
    if text.is_empty() || utf16_len(text) > MAX_INPUT_CHARS {
        return None;
    }
    let raw_tokens = default_count_tokens(text);
    if raw_tokens == 0 {
        return None;
    }
    let kind = classify_crush_text(text);
    CRUSH_STRATEGIES
        .into_iter()
        .filter(|strategy| strategy.kind() == kind && is_enabled(*strategy, config, tool_name))
        .find_map(|strategy| {
            let crushed = strategy
                .run(text)
                .filter(|out| !out.is_empty() && out != text)?;
            let new_tokens = default_count_tokens(&crushed);
            let reduction = (raw_tokens as f64 - new_tokens as f64) / raw_tokens as f64;
            let keeps_errors = kind != CrushKind::Log
                || !strategy.lossy()
                || log::error_lines_survive(text, &crushed);
            (reduction >= config.min_reduction && keeps_errors).then_some(CrushOutput {
                text: crushed,
                strategy,
            })
        })
}

fn is_enabled(strategy: CrushStrategy, config: &CrushConfig, tool_name: Option<&str>) -> bool {
    let Some(settings) = config.strategies.get(strategy.id()) else {
        return true;
    };
    settings.enabled
        && !tool_name.is_some_and(|name| {
            settings
                .exclude_tools
                .iter()
                .any(|pattern| match_tool_pattern(name, pattern))
        })
}

/// The decision for one tool result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrushDecision {
    /// Under absorption's gates: left alone.
    Skip,
    /// Crushed under absorption's size gate: no absorb round-trip.
    Crushed,
    /// Still over it (crushed or not): absorption asks the model.
    Distill,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CrushEvaluation {
    pub decision: CrushDecision,
    /// The original for `Skip` and an uncrushable result, else the crushed
    /// text.
    pub text: String,
    pub raw_tokens: u64,
    pub new_tokens: Option<u64>,
    pub reduction: Option<f64>,
    pub strategy: Option<CrushStrategy>,
}

/// The two-tier gate for one tool result: crush it deterministically, and
/// leave to absorption only what is still over its size gate. The same
/// input always gives the same bytes.
pub fn evaluate_tool_result(
    text: &str,
    config: &Config,
    token_count: u64,
    tool_name: Option<&str>,
) -> CrushEvaluation {
    let absorb = &config.absorb;
    let raw_tokens = default_count_tokens(text);
    let limit = config.model_context_limit as f64;
    let skip = CrushEvaluation {
        decision: CrushDecision::Skip,
        text: text.to_owned(),
        raw_tokens,
        new_tokens: None,
        reduction: None,
        strategy: None,
    };
    if absorb.context_threshold_pct > 0.0
        && limit > 0.0
        && (token_count as f64) < absorb.context_threshold_pct * limit
        || raw_tokens < absorb.min_tool_tokens
    {
        return skip;
    }
    let Some(out) = crush_text(text, &config.crush, tool_name) else {
        return CrushEvaluation {
            decision: CrushDecision::Distill,
            ..skip
        };
    };
    let new_tokens = default_count_tokens(&out.text);
    CrushEvaluation {
        decision: if new_tokens < absorb.min_tool_tokens {
            CrushDecision::Crushed
        } else {
            CrushDecision::Distill
        },
        text: out.text,
        raw_tokens,
        new_tokens: Some(new_tokens),
        reduction: Some((raw_tokens as f64 - new_tokens as f64) / raw_tokens as f64),
        strategy: Some(out.strategy),
    }
}

/// The view with its absorption candidates crushed.
#[derive(Clone, Debug, PartialEq)]
pub struct CrushResult {
    pub messages: Vec<Message>,
    /// Crushed under the absorb gate.
    pub crushed_count: usize,
    /// Passed on to absorption, crushed or not.
    pub distilled_count: usize,
}

/// Crushes every absorption candidate of the view; absorption then decides
/// on the crushed sizes in the same request. A stale absorb demand already
/// in a text is dropped with the crush and decided again.
pub fn apply_crush_to_messages(
    messages: Vec<Message>,
    config: &Config,
    token_count: u64,
) -> CrushResult {
    let protected_calls = collect_protected_tool_call_ids(&messages, config);
    let mut result = CrushResult {
        messages: Vec::with_capacity(messages.len()),
        crushed_count: 0,
        distilled_count: 0,
    };
    for message in messages {
        if !crate::absorb::is_candidate_with_protected_calls(&message, config, &protected_calls) {
            result.messages.push(message);
            continue;
        }
        let payload = match message.text.find(ABSORB_PROMPT_MARKER) {
            Some(at) => &message.text[..at],
            None => message.text.as_str(),
        };
        let evaluation =
            evaluate_tool_result(payload, config, token_count, message.tool_name.as_deref());
        match evaluation.decision {
            CrushDecision::Skip => {
                result.messages.push(message);
                continue;
            }
            CrushDecision::Crushed => result.crushed_count += 1,
            CrushDecision::Distill => result.distilled_count += 1,
        }
        result.messages.push(Message {
            text: evaluation.text,
            ..message
        });
    }
    result
}
