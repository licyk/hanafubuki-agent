mod chain;

use serde::{Deserialize, Serialize};

pub(crate) use chain::{next_link, CompressionChainHead};
pub use chain::{AgentCompressionChain, AgentCompressionDelta, AgentCompressionUpdate};

/// A session's hierarchical compression state: the refs minted over its
/// window, the blocks the model folded and the nudge's baselines.
pub use hanafubuki_agent_compression::State as AgentCompressionState;

/// The originals a `hierarchical` session's content store replaced, by ref.
/// A host's history store keeps it in a file of its own, apart from the
/// rollout's records.
pub use hanafubuki_agent_compression::ContentStore as AgentContentStore;

/// The tuning of the `hierarchical` strategy, from
/// `[compaction.hierarchical]`: the facade puts it in each turn's extensions.
/// A turn without it uses the defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHierarchicalCompression {
    /// Where the pressure band of the nudges starts, as a share of the
    /// context window.
    pub max_context_limit_percent: u8,
    /// Where the emergency nudge and the truncation of tool outputs start.
    pub emergency_threshold_percent: u8,
    /// How much the context grows before the next nudge.
    pub nudge_growth_tokens: u64,
    pub preserve_recent_messages: u32,
    pub preserve_recent_tokens: u64,
    pub min_compress_range_chars: u64,
    pub min_summary_length: u64,
    pub max_summary_length: u64,
    pub tiers: bool,
    /// Tools whose calls and results never fold.
    pub protected_tools: Vec<String>,
    /// A large tool result asks the model to distil it with `absorb`, and
    /// is then hidden.
    pub absorb: bool,
    /// A large tool result is replaced on arrival by a placeholder, and
    /// `acp_retrieve` returns the original.
    pub content_store: bool,
    /// JSON, code and logs in large tool results are folded
    /// deterministically before `absorb` is asked for; only with `absorb`.
    pub crush: bool,
    /// The model records persistent reminders with `acp_rule`, shown in the
    /// instructions.
    pub rules: bool,
}

impl Default for AgentHierarchicalCompression {
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
            absorb: false,
            content_store: false,
            crush: false,
            rules: false,
        }
    }
}

impl AgentHierarchicalCompression {
    /// The algorithm's configuration for a window of `context_window_tokens`.
    pub(crate) fn config(
        &self,
        context_window_tokens: u64,
    ) -> hanafubuki_agent_compression::Config {
        let mut config = hanafubuki_agent_compression::Config::new(context_window_tokens);
        config.nudge.max_context_limit_pct = f64::from(self.max_context_limit_percent) / 100.0;
        config.nudge.min_context_limit_pct = config
            .nudge
            .min_context_limit_pct
            .min(config.nudge.max_context_limit_pct);
        let emergency = f64::from(self.emergency_threshold_percent) / 100.0;
        config.nudge.emergency_threshold_pct = emergency.max(config.nudge.max_context_limit_pct);
        config.truncate.threshold = config.nudge.emergency_threshold_pct;
        config.nudge.growth_floor = self.nudge_growth_tokens;
        config.nudge.growth_cap = self.nudge_growth_tokens;
        config.preserve_recent_messages = self.preserve_recent_messages as usize;
        config.preserve_recent_tokens = self.preserve_recent_tokens;
        config.compress.min_compress_range = to_usize(self.min_compress_range_chars);
        config.compress.min_summary_length = to_usize(self.min_summary_length);
        config.compress.max_summary_length = to_usize(self.max_summary_length);
        config.tiers.enabled = self.tiers;
        config.protected_tools = self.protected_tools.clone();
        config.absorb.enabled = self.absorb;
        config.content_store.enabled = self.content_store;
        config.crush.enabled = self.crush;
        config.rules.enabled = self.rules;
        config
    }
}

/// What a session's hierarchical compression has folded, as its snapshot
/// shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentCompressionSummary {
    /// The blocks the requests show in place of what they cover.
    pub active_blocks: usize,
    /// The tokens every fold replaced, cumulative.
    pub tokens_compressed: u64,
}

impl AgentCompressionSummary {
    pub(crate) fn of(state: &AgentCompressionState) -> Self {
        Self {
            active_blocks: state.active_blocks().count(),
            tokens_compressed: state.stats.tokens_compressed,
        }
    }
}

/// The counters of a session's hierarchical compression.
pub use hanafubuki_agent_compression::Stats as AgentCompressionStats;

/// The size of a session's content store.
pub use hanafubuki_agent_compression::ContentStoreStats as AgentContentStoreStats;

/// A session's hierarchical compression as a person reads it (the terminal
/// interface's `/compression`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCompressionStatus {
    /// billion-context's status panel: the context, the nudge and the
    /// compressible ranges, the active blocks by tier.
    pub panel: String,
    /// The cache report, in full (`acp_cache`'s).
    pub cache_report: String,
    pub stats: AgentCompressionStats,
    /// The content store's size, when the session has one.
    pub content_store: Option<AgentContentStoreStats>,
}

/// One range a `compress` call asks to fold, as an interface shows the
/// call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCompressRange {
    /// `mNNNNN` or `bN`.
    pub start_ref: String,
    pub end_ref: String,
    pub topic: Option<String>,
    pub summary: String,
}

/// The ranges of a `compress` call's `arguments`, read as the tool reads
/// them (leniently); none when nothing in them parses.
pub fn compress_call_ranges(arguments: &serde_json::Value) -> Vec<AgentCompressRange> {
    hanafubuki_agent_foundation::panic::catch_panic(|| {
        hanafubuki_agent_compression::parse_compress_args(arguments, None)
            .ranges
            .into_iter()
            .map(|range| AgentCompressRange {
                start_ref: range.start_ref,
                end_ref: range.end_ref,
                topic: range.topic.filter(|topic| !topic.trim().is_empty()),
                summary: range.summary,
            })
            .collect()
    })
    .unwrap_or_default()
}

fn to_usize(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// The persisted state, or none when it cannot be read: the session opens
/// rather than failing, and recovery rebuilds the blocks from the history's
/// `compress` calls.
pub(crate) fn tolerant_compression_state<'de, D>(
    deserializer: D,
) -> Result<Option<AgentCompressionState>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| match serde_json::from_value(value) {
        Ok(state) => Some(state),
        Err(error) => {
            log::warn!("Agent session compression state dropped: it could not be read: {error}");
            None
        }
    }))
}

/// The chain link a record carries; one that cannot be read is
/// [`AgentCompressionUpdate::Unreadable`], which breaks the chain, so the
/// state is rebuilt instead of read wrong.
pub(crate) fn tolerant_compression_update<'de, D>(
    deserializer: D,
) -> Result<Option<AgentCompressionUpdate>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.map(|value| {
        serde_json::from_value(value).unwrap_or_else(|error| {
            log::warn!("Agent session compression link could not be read: {error}");
            AgentCompressionUpdate::Unreadable
        })
    }))
}
