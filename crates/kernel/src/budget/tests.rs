use super::*;

#[test]
fn default_policy_is_valid_and_below_platform_ceilings() {
    assert_eq!(
        AgentTurnBudgetPolicy::default().validate(),
        Ok(AgentTurnBudgetPolicy::default())
    );
}

#[test]
fn invalid_zero_and_above_ceiling_values_are_rejected() {
    let zero = AgentTurnBudgetPolicy {
        maximum_tool_invocations: Some(0),
        ..AgentTurnBudgetPolicy::default()
    };
    assert!(matches!(
        zero.validate(),
        Err(AgentBudgetError::InvalidPolicy {
            resource: "maximum tool invocations",
            ..
        })
    ));

    let excessive = AgentTurnBudgetPolicy {
        maximum_in_flight_tools: ABSOLUTE_MAX_IN_FLIGHT_TOOLS + 1,
        ..AgentTurnBudgetPolicy::default()
    };
    assert!(matches!(
        excessive.validate(),
        Err(AgentBudgetError::InvalidPolicy {
            resource: "maximum in-flight tools",
            ..
        })
    ));
}

#[test]
fn ledger_enforces_cumulative_resources_without_sampling_count_limit() {
    let policy = AgentTurnBudgetPolicy {
        maximum_cumulative_model_input_tokens: Some(10),
        maximum_cumulative_model_output_tokens: Some(5),
        ..AgentTurnBudgetPolicy::default()
    };
    let mut ledger = AgentTurnBudgetLedger::default();
    ledger.record_model_usage(5, 2, policy).unwrap();
    ledger.record_model_usage(5, 3, policy).unwrap();
    assert_eq!(ledger.sampling_admissions, 2);
    assert!(matches!(
        ledger.record_model_usage(1, 0, policy),
        Err(AgentBudgetError::Exhausted {
            resource: "cumulative model input tokens",
            limit: 10
        })
    ));
}

#[test]
fn failed_reservation_does_not_partially_mutate_ledger() {
    let policy = AgentTurnBudgetPolicy {
        maximum_tool_invocations: Some(2),
        ..AgentTurnBudgetPolicy::default()
    };
    let mut ledger = AgentTurnBudgetLedger::default();
    ledger.record_tool_invocations(2, policy).unwrap();
    let before = ledger;
    assert!(ledger.record_tool_invocations(1, policy).is_err());
    assert_eq!(ledger, before);
}

#[test]
fn hook_pending_input_and_compaction_resources_share_the_turn_ledger() {
    let policy = AgentTurnBudgetPolicy {
        maximum_hook_fragment_bytes: Some(8),
        maximum_pending_input_items: 2,
        ..AgentTurnBudgetPolicy::default()
    };
    let mut ledger = AgentTurnBudgetLedger::default();
    ledger.record_hook_fragment_bytes(8, policy).unwrap();
    ledger.admit_pending_input_items(2, policy).unwrap();
    ledger.consume_pending_input_items(1).unwrap();
    ledger.record_compaction().unwrap();
    assert_eq!(
        ledger,
        AgentTurnBudgetLedger {
            hook_fragment_bytes: 8,
            pending_input_items: 1,
            compactions: 1,
            ..AgentTurnBudgetLedger::default()
        }
    );
    assert!(ledger.record_hook_fragment_bytes(1, policy).is_err());
    assert!(ledger.admit_pending_input_items(2, policy).is_err());
    assert!(ledger.consume_pending_input_items(2).is_err());
}

#[test]
fn reset_cumulative_model_usage_clears_only_the_cumulative_token_counters() {
    let policy = AgentTurnBudgetPolicy::default();
    let mut ledger = AgentTurnBudgetLedger::default();
    ledger.record_model_usage(3, 2, policy).unwrap();
    ledger.record_compaction().unwrap();
    ledger.reset_cumulative_model_usage();
    assert_eq!(
        ledger,
        AgentTurnBudgetLedger {
            sampling_admissions: 1,
            compactions: 1,
            ..AgentTurnBudgetLedger::default()
        }
    );
}

#[test]
fn default_budget_keeps_accounting_without_cumulative_limits() {
    let policy = AgentTurnBudgetPolicy::default();
    let mut ledger = AgentTurnBudgetLedger::default();
    for _ in 0..300 {
        ledger.record_model_usage(200_000, 10_000, policy).unwrap();
        ledger.record_tool_invocations(1, policy).unwrap();
        ledger.record_tool_output_bytes(1_000_000, policy).unwrap();
        ledger.record_hook_fragment_bytes(100_000, policy).unwrap();
    }
    assert_eq!(ledger.cumulative_model_input_tokens, 60_000_000);
    assert_eq!(ledger.cumulative_model_output_tokens, 3_000_000);
    assert_eq!(ledger.tool_invocations, 300);
    assert_eq!(ledger.tool_output_bytes, 300_000_000);
    assert_eq!(ledger.hook_fragment_bytes, 30_000_000);
    assert_eq!(policy.maximum_consecutive_stalled_steps, None);
}

#[test]
fn explicit_cumulative_budgets_have_no_arbitrary_ceiling() {
    let policy = AgentTurnBudgetPolicy {
        maximum_cumulative_model_input_tokens: Some(u64::MAX),
        maximum_cumulative_model_output_tokens: Some(u64::MAX),
        maximum_tool_invocations: Some(u64::MAX),
        maximum_tool_output_bytes: Some(u64::MAX),
        maximum_hook_fragment_bytes: Some(u64::MAX),
        maximum_consecutive_stalled_steps: Some(u32::MAX),
        ..Default::default()
    };
    assert_eq!(policy.validate(), Ok(policy));
}
