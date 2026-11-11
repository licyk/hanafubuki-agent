use super::*;

#[test]
fn claude_up_to_4_5_and_other_ids_take_manual_thinking() {
    for model in [
        "claude-3-7-sonnet-latest",
        "claude-sonnet-4-20250514",
        "claude-opus-4-0",
        "claude-opus-4-1-20250805",
        "claude-opus-4-5-20251101",
        "claude-sonnet-4-5",
        "claude-haiku-4-5@20251001",
        "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
        "Claude-Sonnet-4.5",
        "deepseek-chat",
        "glm-4.6",
    ] {
        assert!(takes_manual_thinking(model), "{model}");
    }
}

#[test]
fn claude_4_6_and_later_take_adaptive_thinking() {
    for model in [
        "claude-opus-4-6",
        "claude-sonnet-4-6",
        "claude-opus-4-7",
        "claude-opus-4-8",
        "claude-opus-5",
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "claude-haiku-5-5",
        "claude-fable-5-2",
        "claude-mythos-preview",
        "anthropic/claude-opus-5-5",
    ] {
        assert!(!takes_manual_thinking(model), "{model}");
    }
}

#[test]
fn every_effort_has_a_budget_from_the_minimum() {
    let budgets = [
        "minimal",
        "low",
        "medium",
        "high",
        "xhigh",
        "max",
        "vendor.tier",
    ]
    .map(|effort| budget_tokens(&ModelReasoningEffort::parse(effort).unwrap()));
    assert_eq!(
        budgets,
        [1_024, 4_096, 8_192, 16_384, 24_576, 32_000, 8_192]
    );
}

#[test]
fn a_largest_output_too_small_for_a_budget_sends_no_thinking() {
    let mut request = crate::test_support::deterministic_user_request("hello");
    request.model = "claude-sonnet-4-5".into();
    request.reasoning = Some(crate::ModelReasoningConfig {
        effort: ModelReasoningEffort::Low,
        max_output_tokens: Some(4_096),
    });
    let thinking = Thinking::of(&request);
    assert!(thinking.fields.is_empty() && !thinking.replay);
    assert_eq!(thinking.max_tokens, 4_096, "never above the largest output");
}
