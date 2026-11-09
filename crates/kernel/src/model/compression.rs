use serde::Deserialize;

/// A session's hierarchical compression state: the refs minted over its
/// window, the blocks the model folded and the nudge's baselines.
pub use hanafubuki_agent_compression::State as AgentCompressionState;

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
        config
    }
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
