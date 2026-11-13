//! A Provider's refusal of a structured request's rung: its native output
//! (Responses' `text.format`, Chat Completions' `response_format`) or its
//! forced tool (`tool_choice`), recognized narrowly: a broad match would
//! teach the route a lower rung on an unrelated refusal, such as a schema
//! the Provider found invalid. Matched as lowercase substrings.

/// Codes that refuse a parameter, which count only for an error that names
/// the parameter.
const CODES: [&str; 2] = ["unsupported_parameter", "unsupported_value"];

/// The names of native structured output.
const STRUCTURED_OUTPUT: [&str; 5] = [
    "text.format",
    "response_format",
    "json_schema",
    "structured output",
    "structured_output",
];

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

/// Whether an error refuses native structured output: a status of 400 or
/// 422, and a message that names it with a refusal code or wording.
pub(crate) fn is_unsupported_structured_output(
    status: Option<u16>,
    code: Option<&str>,
    message: &str,
) -> bool {
    refuses(status, code, message, |message| {
        STRUCTURED_OUTPUT.iter().any(|name| message.contains(name))
    })
}

/// Whether an error refuses a forced tool: a status of 400 or 422, and a
/// message that names `tool_choice` (or `tools`) with a refusal code or
/// wording.
pub(crate) fn is_unsupported_tool_choice(
    status: Option<u16>,
    code: Option<&str>,
    message: &str,
) -> bool {
    refuses(status, code, message, |message| {
        message.contains("tool_choice") || message.contains("tools")
    })
}

fn refuses(
    status: Option<u16>,
    code: Option<&str>,
    message: &str,
    names: impl Fn(&str) -> bool,
) -> bool {
    if !matches!(status, Some(400 | 422)) {
        return false;
    }
    let message = message.to_ascii_lowercase();
    if !names(&message) {
        return false;
    }
    code.is_some_and(|code| CODES.iter().any(|known| code.eq_ignore_ascii_case(known)))
        || WORDINGS.iter().any(|wording| message.contains(wording))
}

#[cfg(test)]
mod tests;
