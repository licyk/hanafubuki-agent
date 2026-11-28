//! How the context's size is measured from what the Provider reported and
//! what the Agent counted after it.

use super::*;

fn reported(consumed: u64, baseline: Option<u64>) -> AgentModelUsage {
    AgentModelUsage {
        last_response_input_tokens: consumed,
        last_response_output_tokens: 0,
        cumulative_input_tokens: consumed,
        cumulative_output_tokens: 0,
        context_window_tokens: Some(1_000_000),
        context_consumed_tokens: Some(consumed),
        context_reported_tokens: Some(consumed),
        context_estimated_tokens: baseline,
        source: AgentUsageSource::ProviderReported,
        last_response_cache_read_tokens: None,
        last_response_cache_write_tokens: None,
        cumulative_cache_read_tokens: 0,
        cumulative_cache_write_tokens: 0,
        cache_reporting_responses: 0,
    }
}

#[test]
fn a_reported_count_and_its_estimate_add_only_the_growth() {
    assert_eq!(
        reported(500_000, Some(400_000)).calibrated_context_tokens(430_000),
        530_000
    );
    // Nothing grew: the Provider's count stands, an estimate below it adds
    // nothing.
    assert_eq!(
        reported(500_000, Some(400_000)).calibrated_context_tokens(380_000),
        500_000
    );
}

#[test]
fn without_a_baseline_the_larger_of_the_report_and_the_estimate_stands() {
    // A model switch drops the baseline. Counting the estimate alone would
    // understate a model whose tokenizer packs more per token, and count a
    // history the Provider measured at 55% as 80% when the Agent's estimate
    // runs high.
    let switched = reported(550_000, None);
    assert_eq!(switched.calibrated_context_tokens(800_000), 800_000);
    assert_eq!(switched.calibrated_context_tokens(240_000), 550_000);
}

#[test]
fn an_estimated_count_is_the_estimate() {
    let mut estimated = reported(500_000, Some(400_000));
    estimated.source = AgentUsageSource::Estimated;
    assert_eq!(estimated.calibrated_context_tokens(120_000), 120_000);
}

#[test]
fn a_compaction_counts_the_installed_context_alone() {
    // The instalment drops the count of the context it replaced.
    let mut usage = reported(550_000, Some(800_000));
    usage.context_consumed_tokens = Some(240_000);
    usage.context_reported_tokens = None;
    usage.source = AgentUsageSource::Estimated;
    assert_eq!(usage.context_size_reported(), Some(240_000));
    assert_eq!(usage.calibrated_context_tokens(250_000), 250_000);
}
