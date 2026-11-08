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
        maximum_tool_invocations: 0,
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
        maximum_cumulative_model_input_tokens: 10,
        maximum_cumulative_model_output_tokens: 5,
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
        maximum_tool_invocations: 2,
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
        maximum_hook_fragment_bytes: 8,
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
fn proactive_model_usage_limit_tracks_the_threshold_percentage_of_each_limit() {
    let policy = AgentTurnBudgetPolicy {
        maximum_cumulative_model_input_tokens: 100,
        maximum_cumulative_model_output_tokens: 50,
        ..AgentTurnBudgetPolicy::default()
    };
    let mut ledger = AgentTurnBudgetLedger::default();
    assert!(!ledger.proactive_model_usage_limit_reached(policy, 80));
    ledger.record_model_usage(79, 0, policy).unwrap();
    assert!(!ledger.proactive_model_usage_limit_reached(policy, 80));
    ledger.record_model_usage(1, 0, policy).unwrap();
    assert!(ledger.proactive_model_usage_limit_reached(policy, 80));

    let mut output_ledger = AgentTurnBudgetLedger::default();
    output_ledger.record_model_usage(0, 40, policy).unwrap();
    assert!(output_ledger.proactive_model_usage_limit_reached(policy, 80));
    assert!(!output_ledger.proactive_model_usage_limit_reached(policy, 90));
}

#[test]
fn percentage_of_threshold_computes_without_overflow() {
    assert_eq!(percentage_of(1_000, 0), 0);
    assert_eq!(percentage_of(1_000, 80), 800);
    assert_eq!(percentage_of(1_000, 100), 1_000);
    assert_eq!(percentage_of(u64::MAX, 80), 14_757_395_258_967_641_292);
}
