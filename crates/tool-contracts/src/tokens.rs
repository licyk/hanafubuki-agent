//! How the text of a request is counted when no tokenizer is at hand.

/// The bytes of UTF-8 one token is estimated at when a model's own ratio is
/// unknown: about four, as coding agents count.
pub const DEFAULT_BYTES_PER_TOKEN: f64 = 4.0;
