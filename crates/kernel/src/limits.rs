use std::time::Duration;

/// Absolute limits for the first Agent implementation.
///
/// Configurable defaults introduced later must remain at or below these caps.
/// This prevents an unbounded model/tool loop even when a provider repeatedly
/// requests follow-up work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentHardLimits {
    pub maximum_sampling_steps: u32,
    pub maximum_concurrent_tool_calls: usize,
    pub maximum_tool_argument_bytes: usize,
    pub maximum_tool_output_bytes: usize,
    pub maximum_model_output_bytes: usize,
    pub maximum_compaction_summary_bytes: usize,
    pub maximum_compaction_summary_tokens: u64,
    pub maximum_history_items: usize,
    pub maximum_history_bytes: usize,
    pub stream_idle_timeout: Duration,
    pub approval_timeout: Duration,
    pub turn_deadline: Duration,
    pub event_buffer_capacity: usize,
}

pub const FIRST_BATCH_HARD_LIMITS: AgentHardLimits = AgentHardLimits {
    maximum_sampling_steps: 32,
    maximum_concurrent_tool_calls: 8,
    maximum_tool_argument_bytes: 64 * 1024,
    maximum_tool_output_bytes: 256 * 1024,
    maximum_model_output_bytes: 1024 * 1024,
    maximum_compaction_summary_bytes: 256 * 1024,
    maximum_compaction_summary_tokens: 64 * 1024,
    maximum_history_items: 1_024,
    maximum_history_bytes: 4 * 1024 * 1024,
    stream_idle_timeout: Duration::from_secs(120),
    approval_timeout: Duration::from_secs(15 * 60),
    turn_deadline: Duration::from_secs(30 * 60),
    event_buffer_capacity: 1_024,
};
