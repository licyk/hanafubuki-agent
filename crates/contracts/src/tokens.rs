//! The Agent's one token estimate: about four bytes of UTF-8 per token. The
//! kernel's request and history estimates and every plugin budget measured
//! in tokens (the Skill catalog, MCP resource listings, memory evidence) use
//! it, so a budget a plugin fits to and the estimate the kernel admits by
//! count the same way. Provider-reported usage, when present, replaces it.

const BYTES_PER_TOKEN: usize = 4;

/// The tokens `bytes` of text are estimated at, rounded up.
pub fn estimate_tokens(bytes: usize) -> u64 {
    u64::try_from(bytes.div_ceil(BYTES_PER_TOKEN)).unwrap_or(u64::MAX)
}

/// The bytes of text that `tokens` allow.
pub fn token_budget_bytes(tokens: u64) -> usize {
    usize::try_from(tokens)
        .unwrap_or(usize::MAX)
        .saturating_mul(BYTES_PER_TOKEN)
}
