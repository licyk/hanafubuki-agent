use std::time::Duration;

/// Resource bounds and optional whole-turn limits supplied by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentHardLimits {
    /// No sampling-count limit when absent.
    pub maximum_sampling_steps: Option<u32>,
    pub maximum_concurrent_tool_calls: usize,
    pub maximum_tool_argument_bytes: usize,
    pub maximum_tool_output_bytes: usize,
    pub maximum_model_output_bytes: usize,
    /// The most text one reasoning item shows and keeps. Thinking is not
    /// model output for `maximum_model_output_bytes`: past this the rest of
    /// the item is dropped and the kept text is marked truncated.
    pub maximum_reasoning_text_bytes: usize,
    pub maximum_compaction_summary_bytes: usize,
    pub maximum_compaction_summary_tokens: u64,
    /// No item-count limit when absent; retained history bytes remain bounded.
    pub maximum_history_items: Option<usize>,
    pub maximum_history_bytes: usize,
    pub stream_idle_timeout: Duration,
    pub approval_timeout: Duration,
    /// No whole-turn deadline when absent; individual operations stay bounded.
    pub turn_deadline: Option<Duration>,
    /// How long the kernel waits for one context, turn admission, tool
    /// source or prompt `prepare` contribution, or for a tool's
    /// `permission`. A call past it is cancelled: the contribution is left
    /// out, the permission check fails the tool call.
    pub plugin_call_deadline: Duration,
    pub event_buffer_capacity: usize,
}

pub const FIRST_BATCH_HARD_LIMITS: AgentHardLimits = AgentHardLimits {
    maximum_sampling_steps: None,
    maximum_concurrent_tool_calls: 8,
    maximum_tool_argument_bytes: 64 * 1024,
    maximum_tool_output_bytes: 256 * 1024,
    maximum_model_output_bytes: 1024 * 1024,
    maximum_reasoning_text_bytes: 256 * 1024,
    maximum_compaction_summary_bytes: 256 * 1024,
    maximum_compaction_summary_tokens: 64 * 1024,
    maximum_history_items: None,
    maximum_history_bytes: 16 * 1024 * 1024,
    stream_idle_timeout: Duration::from_secs(120),
    approval_timeout: Duration::from_secs(15 * 60),
    turn_deadline: None,
    plugin_call_deadline: Duration::from_secs(10),
    event_buffer_capacity: 1_024,
};
