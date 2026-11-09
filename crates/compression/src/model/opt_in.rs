// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/types.ts`, `kernel/src/config.ts`, `kernel/src/absorb.ts`,
// `kernel/src/ccr.ts`, `kernel/src/crush.ts`, `kernel/src/rules.ts` and
// `kernel/src/filter/types.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the tool names are fixed (`absorb`,
// `acp_retrieve`), so they are not settings; every section is present with
// the kernel's defaults instead of optional; a record's time is the
// caller's.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Instant tool-result absorption: a large tool result asks the model to
/// distil it with `absorb`, and the pair is then hidden.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AbsorbConfig {
    pub enabled: bool,
    /// Results from this many tokens on are asked for; 0 asks for all.
    pub min_tool_tokens: u64,
    /// Ask only from this share of the window on; 0 is the size gate alone.
    pub context_threshold_pct: f64,
    /// Tool patterns never asked for, whatever their protection.
    pub exclude_tools: Vec<String>,
    /// Only results with images, whatever their size.
    pub media_only: bool,
}

impl Default for AbsorbConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_tool_tokens: 4000,
            context_threshold_pct: 0.0,
            exclude_tools: Vec::new(),
            media_only: false,
        }
    }
}

/// Deterministic compaction of large tool results, before absorption asks
/// the model; it needs absorption on.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CrushConfig {
    pub enabled: bool,
    /// The share of tokens a compaction must save to be used, in `(0, 1]`.
    pub min_reduction: f64,
    /// By strategy id (`json-fold`, `code-trim`, `log-select`).
    pub strategies: BTreeMap<String, CrushStrategyOverride>,
}

impl Default for CrushConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_reduction: 0.1,
            strategies: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CrushStrategyOverride {
    pub enabled: bool,
    /// Tool patterns this strategy leaves alone.
    pub exclude_tools: Vec<String>,
}

impl Default for CrushStrategyOverride {
    fn default() -> Self {
        Self {
            enabled: true,
            exclude_tools: Vec::new(),
        }
    }
}

/// The content store: a large tool result is replaced once, when it
/// arrives, by a placeholder, and `acp_retrieve` returns the original.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContentStoreConfig {
    pub enabled: bool,
    /// Results from this many tokens on are stored; 0 stores all.
    pub min_tool_tokens: u64,
    /// Tool patterns never stored, whatever their protection.
    pub exclude_tools: Vec<String>,
    /// The longest preview a placeholder shows, in UTF-16 code units.
    pub max_head_chars: usize,
    /// Originals from this many tokens on are retrieved as a file.
    pub retrieve_inline_tokens: u64,
}

impl Default for ContentStoreConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_tool_tokens: 4000,
            exclude_tools: Vec::new(),
            max_head_chars: 96,
            retrieve_inline_tokens: 4000,
        }
    }
}

/// Persistent reminders the model records with `acp_rule`.
#[derive(Clone, Debug, PartialEq)]
pub struct RulesConfig {
    pub enabled: bool,
    pub max_rules: usize,
    /// In UTF-16 code units.
    pub max_rule_chars: usize,
}

impl Default for RulesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_rules: 50,
            max_rule_chars: 300,
        }
    }
}

/// The host's message filters ([`MessageFilter`](crate::MessageFilter));
/// there are no built-in ones.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MessageFiltersConfig {
    pub enabled: bool,
    /// A filter turned off by name; one not listed runs.
    pub filters: BTreeMap<String, bool>,
}

/// One absorbed tool result: its call and result are hidden, and the
/// model's `absorb` call is the record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AbsorbRecord {
    pub tool_call_id: String,
    /// Empty when the call is not in the history.
    pub call_message_id: String,
    pub result_message_id: String,
    pub absorb_call_id: Option<String>,
    pub summary: String,
    pub tokens_reclaimed: u64,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
}

/// A persistent reminder; ids (`rule1`, ...) are never reissued.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuleRecord {
    pub id: String,
    pub text: String,
}
