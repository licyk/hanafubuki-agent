//! The Agent's one token estimate: about four bytes of UTF-8 per token. The
//! kernel's request and history estimates and every plugin budget measured
//! in tokens (the Skill catalog, MCP resource listings, memory evidence) use
//! it, so a budget a plugin fits to and the estimate the kernel admits by
//! count the same way. Provider-reported usage, when present, replaces it.
//!
//! A model may know its own ratio, which its metadata carries as
//! `bytesPerToken`: a tokenizer that packs more text per token than four
//! bytes makes the same text fewer tokens. The kernel's request estimate is
//! the one that admits compaction, so it counts at that ratio
//! ([`estimate_tokens_with`]); every other estimate here counts at the
//! default, as it always has.

/// The bytes of UTF-8 one token is estimated at when a model's own ratio is
/// unknown.
pub use hanafubuki_agent_tool_contracts::DEFAULT_BYTES_PER_TOKEN;

/// The tokens `bytes` of text are estimated at, rounded up.
pub fn estimate_tokens(bytes: usize) -> u64 {
    estimate_tokens_with(bytes, DEFAULT_BYTES_PER_TOKEN)
}

/// The tokens `bytes` of text are estimated at with `bytes_per_token` bytes
/// of UTF-8 per token, rounded up. A ratio that is not positive, or not
/// finite, estimates as the default does.
pub fn estimate_tokens_with(bytes: usize, bytes_per_token: f64) -> u64 {
    let ratio = if bytes_per_token.is_finite() && bytes_per_token > 0.0 {
        bytes_per_token
    } else {
        DEFAULT_BYTES_PER_TOKEN
    };
    u64::try_from((bytes as f64 / ratio).ceil() as u128).unwrap_or(u64::MAX)
}

/// The bytes of text that `tokens` allow.
pub fn token_budget_bytes(tokens: u64) -> usize {
    usize::try_from(tokens)
        .unwrap_or(usize::MAX)
        .saturating_mul(4)
}
