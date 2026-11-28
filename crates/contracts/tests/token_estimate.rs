//! The one token estimate, at the default ratio and at a model's own.

use hanafubuki_agent_contracts::{
    estimate_tokens, estimate_tokens_with, token_budget_bytes, DEFAULT_BYTES_PER_TOKEN,
};

#[test]
fn the_default_counts_four_bytes_to_a_token() {
    assert_eq!(DEFAULT_BYTES_PER_TOKEN, 4.0);
    for (bytes, tokens) in [(0, 0), (1, 1), (4, 1), (5, 2), (8_000, 2_000)] {
        assert_eq!(estimate_tokens(bytes), tokens, "{bytes} bytes");
        assert_eq!(estimate_tokens_with(bytes, 4.0), tokens, "{bytes} bytes");
    }
    assert_eq!(token_budget_bytes(2_000), 8_000);
}

#[test]
fn a_models_own_ratio_counts_the_same_text() {
    // 8 kB of text is 2 000 tokens at four bytes, 2 500 at 3.2, 1 000 at 8.
    assert_eq!(estimate_tokens_with(8_000, 3.2), 2_500);
    assert_eq!(estimate_tokens_with(8_000, 8.0), 1_000);
    // Rounded up: a partial token is a token.
    assert_eq!(estimate_tokens_with(3, 2.0), 2);
}

#[test]
fn a_ratio_that_cannot_count_estimates_as_the_default_does() {
    for ratio in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(estimate_tokens_with(8_000, ratio), 2_000, "{ratio}");
    }
}
