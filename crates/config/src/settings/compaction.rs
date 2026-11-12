//! `[compaction]`: which strategy reduces a session's context, and the
//! tuning of the strategies that have any.

use hanafubuki_agent_foundation::settings::{
    SettingChoice, SettingCondition, SettingKind, SettingSpec, SettingValue,
};
use serde::{Deserialize, Serialize};

use crate::sections::{check_specs, described, no_rule, SettingsRead, SettingsSection};

const COMPACTION: &str = "compaction";
const MIN_SUMMARY_LENGTH: &str = "compaction.hierarchical.minSummaryLength";
const MAX_SUMMARY_LENGTH: &str = "compaction.hierarchical.maxSummaryLength";

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

    const fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Background => "Background",
            Self::Hierarchical => "Hierarchical",
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
    /// Each message's ref tag shows when it was first seen.
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

pub(crate) fn section() -> SettingsSection {
    let under = |strategy: AgentCompactionStrategy, spec: SettingSpec| SettingSpec {
        visible_if: Some(SettingCondition {
            key: "compaction.strategy".into(),
            equals: SettingValue::Text(strategy.as_str().into()),
        }),
        ..spec
    };
    let background = |spec| under(AgentCompactionStrategy::Background, spec);
    let hierarchical = |spec| under(AgentCompactionStrategy::Hierarchical, spec);
    let advanced = |spec: SettingSpec| SettingSpec {
        advanced: true,
        ..spec
    };
    let percent = || integer(1, 100);
    let tools = || SettingKind::TextList {
        maximum_items: None,
        maximum_item_chars: None,
    };
    SettingsSection::new(
        COMPACTION,
        "Compaction",
        vec![
            described(
                SettingSpec::new(
                    "compaction.strategy",
                    "Compaction strategy",
                    SettingKind::Choice {
                        values: AgentCompactionStrategy::ALL
                            .iter()
                            .map(|strategy| SettingChoice::new(strategy.as_str(), strategy.label()))
                            .collect(),
                    },
                ),
                "What reduces a session's context",
            ),
            background(described(
                SettingSpec::new(
                    "compaction.background.thresholdPercent",
                    "Background threshold (%)",
                    percent(),
                ),
                "The share of the window that starts a compaction in the background",
            )),
            background(described(
                SettingSpec::new(
                    "compaction.background.childSessions",
                    "Child sessions too",
                    SettingKind::Boolean,
                ),
                "Whether child sessions compact in the background too",
            )),
            hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.maxContextLimitPercent",
                    "Fold at (%)",
                    percent(),
                ),
                "Past this share of the window, the model is asked to fold its history at once",
            )),
            hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.emergencyThresholdPercent",
                    "Emergency at (%)",
                    percent(),
                ),
                "Past this share of the window, oversized tool outputs are cut",
            )),
            advanced(hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.nudgeGrowthTokens",
                    "Nudge growth (tokens)",
                    integer(0, u64::MAX),
                ),
                "The model is asked to fold only once a foldable stretch has grown by this many tokens",
            ))),
            advanced(hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.preserveRecentMessages",
                    "Recent messages kept",
                    integer(0, u32::MAX.into()),
                ),
                "The latest messages are kept out of folds",
            ))),
            advanced(hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.preserveRecentTokens",
                    "Recent tokens kept",
                    integer(0, u64::MAX),
                ),
                "The latest tokens are kept out of folds",
            ))),
            advanced(hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.minCompressRangeChars",
                    "Smallest fold (characters)",
                    integer(0, u64::MAX),
                ),
                "The smallest stretch of history worth folding, in characters",
            ))),
            advanced(hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.minSummaryLength",
                    "Shortest summary (characters)",
                    integer(0, u64::MAX),
                ),
                "The shortest summary the model may write; at most the longest",
            ))),
            advanced(hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.maxSummaryLength",
                    "Longest summary (characters)",
                    integer(0, u64::MAX),
                ),
                "The longest summary the model may write",
            ))),
            hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.tiers",
                    "Tiers",
                    SettingKind::Boolean,
                ),
                "Whether older summaries are distilled again, in tiers",
            )),
            hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.protectedTools",
                    "Protected tools",
                    tools(),
                ),
                "Tools whose results are never folded",
            )),
            hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.messageTimestamps",
                    "Message timestamps",
                    SettingKind::Boolean,
                ),
                "Each message's tag shows the local time a request first showed it",
            )),
            hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.absorb",
                    "Absorb",
                    SettingKind::Boolean,
                ),
                "A large tool result asks the model to distil it",
            )),
            hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.contentStore",
                    "Content store",
                    SettingKind::Boolean,
                ),
                "A large tool result is replaced by a placeholder the model can retrieve",
            )),
            hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.crush",
                    "Crush",
                    SettingKind::Boolean,
                ),
                "JSON, code and logs in large tool results are folded before absorbing; only with absorb",
            )),
            hierarchical(described(
                SettingSpec::new(
                    "compaction.hierarchical.rules",
                    "Rules",
                    SettingKind::Boolean,
                ),
                "The model records lasting reminders shown in every request's instructions",
            )),
        ],
    )
}

/// An integer from `minimum` to `maximum`, as far as a setting's bound
/// reaches.
fn integer(minimum: i64, maximum: u64) -> SettingKind {
    SettingKind::Integer {
        minimum: Some(minimum),
        maximum: Some(i64::try_from(maximum).unwrap_or(i64::MAX)),
    }
}

pub(crate) fn validate_compaction(settings: &AgentCompactionSettings) -> Result<(), String> {
    check_specs(COMPACTION, settings)?;
    validate_summary_lengths(&settings.hierarchical)
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

/// Each compaction setting falls back on its own; a pair of summary lengths
/// that does not hold takes the pair below.
pub(crate) fn decode_compaction(
    read: &mut SettingsRead<'_>,
    below: &AgentCompactionSettings,
) -> AgentCompactionSettings {
    let mut settings = read.read(COMPACTION, below, no_rule);
    if validate_summary_lengths(&settings.hierarchical).is_err() {
        let hierarchical = &mut settings.hierarchical;
        hierarchical.min_summary_length = below.hierarchical.min_summary_length;
        hierarchical.max_summary_length = below.hierarchical.max_summary_length;
        read.fields().invalid(
            MIN_SUMMARY_LENGTH,
            Some("minSummaryLength is above maxSummaryLength; both take their defaults"),
        );
        read.unmark(MIN_SUMMARY_LENGTH);
        read.unmark(MAX_SUMMARY_LENGTH);
    }
    settings
}
