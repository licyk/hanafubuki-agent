use super::*;

fn usage(cache_read_tokens: Option<u64>) -> AgentModelUsage {
    AgentModelUsage {
        last_response_input_tokens: 2_000,
        last_response_output_tokens: 10,
        cumulative_input_tokens: 2_000,
        cumulative_output_tokens: 10,
        context_window_tokens: None,
        context_consumed_tokens: Some(2_000),
        source: AgentUsageSource::ProviderReported,
        last_response_cache_read_tokens: cache_read_tokens,
        last_response_cache_write_tokens: Some(300),
        cumulative_cache_read_tokens: cache_read_tokens.unwrap_or(0),
        cumulative_cache_write_tokens: 300,
        cache_reporting_responses: 1,
    }
}

fn report(usage: AgentModelUsage) -> String {
    cache_report(
        Some(usage),
        &AgentCompressionState::default(),
        CacheDetail::Summary,
        1_000,
        0,
    )
}

/// The cache reads the provider reported are the report's hits; a write is
/// not one.
#[test]
fn the_cache_report_counts_the_reported_cache_reads_as_hits() {
    let hit = report(usage(Some(1_500)));
    assert!(hit.contains("total input    2000 tok"), "{hit}");
    assert!(hit.contains("total cached   1500 tok  (hit 75.0%"), "{hit}");

    let unreported = report(usage(None));
    assert!(
        unreported.contains("total cached   0 tok  (hit 0.0%"),
        "{unreported}"
    );
}
