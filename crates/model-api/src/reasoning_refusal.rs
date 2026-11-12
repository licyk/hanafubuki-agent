//! A Provider's refusal of the reasoning parameter a request carries
//! (Responses' `reasoning`, Chat Completions' `reasoning_effort`, Anthropic's
//! `thinking` and `output_config.effort`), recognized narrowly: a broad
//! match would retry and teach the route "no reasoning" on an unrelated
//! refusal. Matched as lowercase substrings.

/// Codes that refuse a parameter, which count only for an error that names
/// a reasoning parameter.
const CODES: [&str; 2] = ["unsupported_parameter", "unsupported_value"];

/// The names of the reasoning parameters; `effort` also covers
/// `reasoning_effort` and `reasoning.effort`. The Agent sends Anthropic's
/// `output_config` for its effort alone.
const PARAMETERS: [&str; 4] = ["reasoning", "thinking", "effort", "output_config"];

/// Wordings that refuse a parameter.
const WORDINGS: [&str; 11] = [
    "unsupported",
    "not supported",
    "does not support",
    "doesn't support",
    "do not support",
    "unknown",
    "unrecognized",
    "unrecognised",
    "not allowed",
    "not permitted",
    "unexpected keyword",
];

/// A field of an assistant message, not a reasoning parameter: a refusal
/// that names it (DeepSeek's, of a message sent without it) is about the
/// history, and sending no reasoning parameter would not change it.
const REASONING_CONTENT: &str = "reasoning_content";

/// Whether an error refuses the request's reasoning parameter: a status of
/// 400 or 422 (an in-band error, without one, never counts), and a message
/// that names a reasoning parameter with a refusal code or wording, and not
/// `reasoning_content`.
pub(crate) fn is_unsupported_reasoning(
    status: Option<u16>,
    code: Option<&str>,
    message: &str,
) -> bool {
    if !matches!(status, Some(400 | 422)) {
        return false;
    }
    let message = message.to_ascii_lowercase();
    if message.contains(REASONING_CONTENT)
        || !PARAMETERS
            .iter()
            .any(|parameter| message.contains(parameter))
    {
        return false;
    }
    code.is_some_and(|code| CODES.iter().any(|known| code.eq_ignore_ascii_case(known)))
        || WORDINGS.iter().any(|wording| message.contains(wording))
}

#[cfg(test)]
mod tests;
