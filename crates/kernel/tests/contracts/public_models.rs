use super::*;

#[test]
fn every_core_model_tool_uses_codex_style_non_strict_schema() {
    assert!(AgentCoreTool::ALL
        .into_iter()
        .all(|tool| !tool.model_spec().strict));
}

#[test]
fn first_batch_hard_limits_are_an_explicit_contract() {
    assert_eq!(
        FIRST_BATCH_HARD_LIMITS,
        AgentHardLimits {
            maximum_sampling_steps: 32,
            maximum_concurrent_tool_calls: 8,
            maximum_tool_argument_bytes: 64 * 1024,
            maximum_tool_output_bytes: 256 * 1024,
            maximum_model_output_bytes: 1024 * 1024,
            maximum_reasoning_text_bytes: 256 * 1024,
            maximum_compaction_summary_bytes: 256 * 1024,
            maximum_compaction_summary_tokens: 64 * 1024,
            maximum_history_items: 1024,
            maximum_history_bytes: 4 * 1024 * 1024,
            stream_idle_timeout: Duration::from_secs(120),
            approval_timeout: Duration::from_secs(15 * 60),
            turn_deadline: Duration::from_secs(30 * 60),
            event_buffer_capacity: 1024,
        }
    );
}
