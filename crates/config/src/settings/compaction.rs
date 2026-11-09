//! `[compaction]`: which strategy reduces a session's context, and the
//! tuning of the strategies that have any.

use serde::{Deserialize, Serialize};

use crate::fields::{read_member, read_object, Fields};

/// What starts a reduction of a session's context. Every strategy keeps
/// the standard compaction (`/compact`, the model's `new_context`, the
/// retry after a provider's context error) as its floor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCompactionStrategy {
    /// The model policy's threshold compacts in line.
    #[default]
    Standard,
    /// A lower threshold compacts in the background.
    Background,
    /// The model folds the history into summaries it writes.
    Hierarchical,
}

impl AgentCompactionStrategy {
    pub const ALL: &'static [Self] = &[Self::Standard, Self::Background, Self::Hierarchical];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Background => "background",
            Self::Hierarchical => "hierarchical",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentCompactionSettings {
    pub strategy: AgentCompactionStrategy,
    pub background: AgentBackgroundCompactionSettings,
    pub hierarchical: AgentHierarchicalCompactionSettings,
}

/// `[compaction.background]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentBackgroundCompactionSettings {
    /// The share of the context window that starts a compaction in the
    /// background; at or above the model policy's
    /// `autoCompactThresholdPercent` it never starts one.
    pub threshold_percent: u8,
    /// Whether child sessions compact in the background too; when not, they
    /// run `standard`.
    pub child_sessions: bool,
}

impl Default for AgentBackgroundCompactionSettings {
    fn default() -> Self {
        Self {
            threshold_percent: 65,
            child_sessions: true,
        }
    }
}

/// `[compaction.hierarchical]`, after billion-context's `compress.*`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentHierarchicalCompactionSettings {
    /// Where the pressure band of the nudges starts, as a share of the
    /// context window.
    pub max_context_limit_percent: u8,
    /// Where the emergency nudge and truncation start.
    pub emergency_threshold_percent: u8,
    pub nudge_growth_tokens: u64,
    pub preserve_recent_messages: u32,
    pub preserve_recent_tokens: u64,
    pub min_compress_range_chars: u64,
    pub min_summary_length: u64,
    pub max_summary_length: u64,
    pub tiers: bool,
    /// Tools whose calls are never folded.
    pub protected_tools: Vec<String>,
    pub message_timestamps: bool,
    pub absorb: bool,
    pub content_store: bool,
    pub crush: bool,
    pub rules: bool,
}

impl Default for AgentHierarchicalCompactionSettings {
    fn default() -> Self {
        Self {
            max_context_limit_percent: 75,
            emergency_threshold_percent: 95,
            nudge_growth_tokens: 50_000,
            preserve_recent_messages: 3,
            preserve_recent_tokens: 5_000,
            min_compress_range_chars: 5_000,
            min_summary_length: 50,
            max_summary_length: 20_000,
            tiers: true,
            protected_tools: Vec::new(),
            message_timestamps: true,
            absorb: false,
            content_store: false,
            crush: false,
            rules: false,
        }
    }
}

pub(crate) fn validate_compaction(settings: &AgentCompactionSettings) -> Result<(), String> {
    validate_background(&settings.background)?;
    validate_hierarchical(&settings.hierarchical)?;
    validate_summary_lengths(&settings.hierarchical)
}

fn validate_background(settings: &AgentBackgroundCompactionSettings) -> Result<(), String> {
    if !(1..=100).contains(&settings.threshold_percent) {
        return Err("compaction.background.thresholdPercent is 1 to 100".to_string());
    }
    Ok(())
}

fn validate_hierarchical(settings: &AgentHierarchicalCompactionSettings) -> Result<(), String> {
    for (field, percent) in [
        ("maxContextLimitPercent", settings.max_context_limit_percent),
        (
            "emergencyThresholdPercent",
            settings.emergency_threshold_percent,
        ),
    ] {
        if !(1..=100).contains(&percent) {
            return Err(format!("compaction.hierarchical.{field} is 1 to 100"));
        }
    }
    Ok(())
}

/// The one rule between two fields, checked once both are read: either may
/// be written first.
fn validate_summary_lengths(settings: &AgentHierarchicalCompactionSettings) -> Result<(), String> {
    if settings.min_summary_length > settings.max_summary_length {
        return Err(
            "compaction.hierarchical.minSummaryLength is at most maxSummaryLength".to_string(),
        );
    }
    Ok(())
}

/// Each compaction setting falls back on its own, against the bounds the
/// whole table is validated with.
pub(crate) fn decode_compaction(fields: &mut Fields<'_>, settings: &mut AgentCompactionSettings) {
    settings.strategy = fields.read("strategy", settings.strategy);
    settings.background = read_object(
        fields,
        "background",
        &settings.background,
        decode_background,
    );
    settings.hierarchical = read_object(
        fields,
        "hierarchical",
        &settings.hierarchical,
        decode_hierarchical,
    );
}

fn decode_background(fields: &mut Fields<'_>, settings: &mut AgentBackgroundCompactionSettings) {
    let validate = validate_background;
    read_member(
        fields,
        settings,
        "thresholdPercent",
        |s| &mut s.threshold_percent,
        validate,
    );
    read_member(
        fields,
        settings,
        "childSessions",
        |s| &mut s.child_sessions,
        validate,
    );
}

fn decode_hierarchical(
    fields: &mut Fields<'_>,
    settings: &mut AgentHierarchicalCompactionSettings,
) {
    let validate = validate_hierarchical;
    let below = (settings.min_summary_length, settings.max_summary_length);
    read_member(
        fields,
        settings,
        "maxContextLimitPercent",
        |s| &mut s.max_context_limit_percent,
        validate,
    );
    read_member(
        fields,
        settings,
        "emergencyThresholdPercent",
        |s| &mut s.emergency_threshold_percent,
        validate,
    );
    read_member(
        fields,
        settings,
        "nudgeGrowthTokens",
        |s| &mut s.nudge_growth_tokens,
        validate,
    );
    read_member(
        fields,
        settings,
        "preserveRecentMessages",
        |s| &mut s.preserve_recent_messages,
        validate,
    );
    read_member(
        fields,
        settings,
        "preserveRecentTokens",
        |s| &mut s.preserve_recent_tokens,
        validate,
    );
    read_member(
        fields,
        settings,
        "minCompressRangeChars",
        |s| &mut s.min_compress_range_chars,
        validate,
    );
    read_member(
        fields,
        settings,
        "minSummaryLength",
        |s| &mut s.min_summary_length,
        validate,
    );
    read_member(
        fields,
        settings,
        "maxSummaryLength",
        |s| &mut s.max_summary_length,
        validate,
    );
    read_member(fields, settings, "tiers", |s| &mut s.tiers, validate);
    read_member(
        fields,
        settings,
        "protectedTools",
        |s| &mut s.protected_tools,
        validate,
    );
    read_member(
        fields,
        settings,
        "messageTimestamps",
        |s| &mut s.message_timestamps,
        validate,
    );
    read_member(fields, settings, "absorb", |s| &mut s.absorb, validate);
    read_member(
        fields,
        settings,
        "contentStore",
        |s| &mut s.content_store,
        validate,
    );
    read_member(fields, settings, "crush", |s| &mut s.crush, validate);
    read_member(fields, settings, "rules", |s| &mut s.rules, validate);
    if validate_summary_lengths(settings).is_err() {
        (settings.min_summary_length, settings.max_summary_length) = below;
        fields.invalid(
            "minSummaryLength",
            Some("minSummaryLength is above maxSummaryLength; both take their defaults"),
        );
    }
}
