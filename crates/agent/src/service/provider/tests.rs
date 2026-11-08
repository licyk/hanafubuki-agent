use super::*;
use hanafubuki_agent_config::{
    AgentInputCapabilitiesSettings, AgentModelPolicySettings, AgentRemoteCompactionCapability,
};
use hanafubuki_agent_kernel::ModelReasoningEffort;

fn responses_only_policy() -> AgentModelPolicySettings {
    AgentModelPolicySettings {
        model: "fixture-model".into(),
        reasoning_effort: Some(ModelReasoningEffort::High),
        input_capabilities: AgentInputCapabilitiesSettings::default(),
        context_window_tokens: Some(128_000),
        auto_compaction_enabled: true,
        auto_compact_threshold_percent: 70,
        remote_compaction: AgentRemoteCompactionCapability::ResponsesCompact,
        turn_budget: None,
    }
}

/// The facade resolves the shared policy without narrowing it: the kernel's
/// session config normalization is the one place a protocol drops what it
/// cannot carry (see its validation tests).
#[test]
fn the_policy_is_resolved_unchanged_for_the_kernel_to_narrow() {
    let resolved = resolve_model_policy(&responses_only_policy());

    assert_eq!(resolved.reasoning_effort, Some(ModelReasoningEffort::High));
    assert_eq!(
        resolved.compaction_capability,
        hanafubuki_agent_kernel::ModelCompactionCapability::ResponsesCompact
    );
    assert_eq!(resolved.context_window_tokens, Some(128_000));
    assert!(resolved.auto_compaction_enabled);
    assert_eq!(resolved.auto_compact_threshold_percent, 70);
}
