//! A Provider's refusal of a request too long for the model's context
//! window: whether an error says so, and the window it names. The wordings
//! and the limit's patterns are Hoshimi's (`core/failure.py`), matched as
//! lowercase substrings.

/// Codes that say so whatever else the error says.
const CODES: [&str; 3] = [
    "context_length_exceeded",
    "context_window_exceeded",
    "prompt_too_long",
];

/// Wordings that say so in a refusal. Bare "context length" and "context
/// window" are left out (an unrelated refusal may name the window), and so
/// is "request too large", which is also how a rate limit per minute and a
/// size limit in bytes are worded.
const WORDINGS: [&str; 28] = [
    "context_length_exceeded",
    "context_window_exceeded",
    "context window exceeded",
    "maximum context",
    "input exceeds the context window",
    "exceeds the context length",
    "exceed the context limit",
    "exceeds the context limit",
    "exceeded the context limit",
    "exceed context limit",
    "exceeds context limit",
    "exceeded context limit",
    "prompt is too long",
    "input is too long",
    "too many tokens",
    "too many input tokens",
    "token limit exceeded",
    "token_limit_exceeded",
    "input token count",
    "exceeds the maximum number of tokens",
    "maximum input length",
    "maximum prompt length",
    "上下文长度",
    "上下文窗口",
    "超出最大长度",
    "超过最大长度",
    "超过了最大",
    "输入过长",
];

/// What a rate limit or a quota says, which no overflow does: such an
/// error never counts, whatever else it says ("Request too large for … on
/// tokens per min").
const QUOTA_MARKERS: [&str; 10] = [
    "rate limit",
    "rate_limit",
    "ratelimit",
    "quota",
    "tokens per min",
    "requests per min",
    "too many requests",
    "限流",
    "限速",
    "额度",
];

/// The context windows a refusal is believed to teach: a window it names
/// outside them is taken for another number.
pub const PLAUSIBLE_CONTEXT_WINDOW: std::ops::RangeInclusive<u64> = 1_024..=20_000_000;

/// Whether an error with `status` (none: an error in the stream), `code`
/// and `message` says the request does not fit the model's context window.
/// Only a refusal (400, 413 or 422) or an in-band error is read, its code
/// and message together; a 429 or a rate limit or quota never counts.
pub(crate) fn is_context_window_exceeded(
    status: Option<u16>,
    code: Option<&str>,
    message: &str,
) -> bool {
    if status == Some(429) {
        return false;
    }
    let code = code.map(str::to_ascii_lowercase).unwrap_or_default();
    if CODES.contains(&code.as_str()) {
        return true;
    }
    if status.is_some_and(|status| !matches!(status, 400 | 413 | 422)) {
        return false;
    }
    let text = format!("{code} {}", message.to_ascii_lowercase());
    if QUOTA_MARKERS.iter().any(|marker| text.contains(marker)) {
        return false;
    }
    WORDINGS.iter().any(|wording| text.contains(wording))
        || exceeds_a_length_limit(&text)
        || token_count_exceeded(&text)
}

/// The context window a refusal names ("maximum context length is 131072
/// tokens"), when it names one within 1024..=20,000,000 tokens. The
/// patterns are tried in turn, each at its first match, and a value out of
/// bounds moves on to the next pattern. Commas in a number are ignored.
pub fn context_limit_in(message: &str) -> Option<u64> {
    let text = message.to_ascii_lowercase();
    [
        maximum_length_is(&text),
        context_window_of(&text),
        over_maximum(&text),
        maximum_tokens(&text),
        limit_in_tokens(&text),
    ]
    .into_iter()
    .flatten()
    .find(|value| PLAUSIBLE_CONTEXT_WINDOW.contains(value))
}

/// "maximum context length is N" (and prompt or input length).
fn maximum_length_is(text: &str) -> Option<u64> {
    [
        "maximum context length is",
        "maximum prompt length is",
        "maximum input length is",
    ]
    .iter()
    .find_map(|marker| number_after(text, marker, &[]))
}

/// "context window of N", "context length is N", "context limit of N".
fn context_window_of(text: &str) -> Option<u64> {
    [
        "context window of",
        "context window is",
        "context length of",
        "context length is",
        "context limit of",
        "context limit is",
    ]
    .iter()
    .find_map(|marker| number_after(text, marker, &[]))
}

/// "N tokens > M maximum": M.
fn over_maximum(text: &str) -> Option<u64> {
    text.match_indices('>').find_map(|(at, _)| {
        let (value, rest) = leading_number(text[at + 1..].trim_start())?;
        rest.trim_start().starts_with("maximum").then_some(value)
    })
}

/// "maximum number of tokens allowed (N)", "maximum input tokens: N",
/// "maximum allowed tokens is N".
fn maximum_tokens(text: &str) -> Option<u64> {
    [
        "maximum number of tokens allowed",
        "maximum allowed tokens",
        "maximum input tokens",
        "maximum input token",
    ]
    .iter()
    .find_map(|marker| number_after(text, marker, &["is", "(", ":"]))
}

/// "limit … N tokens" or "最大 … N tokens": a number of four characters or
/// more (digits and commas) followed by "tok", with at most twelve
/// characters and no digit between it and the marker.
fn limit_in_tokens(text: &str) -> Option<u64> {
    ["limit", "最大"].iter().find_map(|marker| {
        text.match_indices(marker).find_map(|(at, _)| {
            let rest = &text[at + marker.len()..];
            let (offset, _) = rest
                .char_indices()
                .take(13)
                .find(|(_, character)| character.is_ascii_digit())?;
            let (value, written, after) = number_at(&rest[offset..])?;
            (written >= 4 && after.trim_start().starts_with("tok")).then_some(value)
        })
    })
}

/// "超出…长度限制", with at most six characters between.
fn exceeds_a_length_limit(message: &str) -> bool {
    message.match_indices("超出").any(|(at, marker)| {
        let rest = &message[at + marker.len()..];
        rest.char_indices()
            .take(7)
            .any(|(offset, _)| rest[offset..].starts_with("长度限制"))
    })
}

/// "token 数量超过", "token数超出" and the like.
fn token_count_exceeded(message: &str) -> bool {
    message.match_indices("token").any(|(at, marker)| {
        let rest = message[at + marker.len()..].trim_start();
        let Some(rest) = rest.strip_prefix('数') else {
            return false;
        };
        let rest = rest.strip_prefix('量').unwrap_or(rest);
        rest.strip_prefix('超')
            .is_some_and(|rest| rest.starts_with('过') || rest.starts_with('出'))
    })
}

/// The first number after an occurrence of `marker` in `text`: past
/// whitespace, one of `separators` if one follows, and whitespace again.
fn number_after(text: &str, marker: &str, separators: &[&str]) -> Option<u64> {
    text.match_indices(marker).find_map(|(at, _)| {
        let rest = text[at + marker.len()..].trim_start();
        let rest = separators
            .iter()
            .find_map(|separator| rest.strip_prefix(separator))
            .unwrap_or(rest);
        leading_number(rest.trim_start()).map(|(value, _)| value)
    })
}

/// The number `text` starts with and what follows it.
fn leading_number(text: &str) -> Option<(u64, &str)> {
    number_at(text).map(|(value, _, after)| (value, after))
}

/// The number `text` starts with (a digit, then digits and commas), how
/// many characters it was written in, and what follows it; none when `text`
/// starts with no digit or the number overflows.
fn number_at(text: &str) -> Option<(u64, usize, &str)> {
    if !text.starts_with(|character: char| character.is_ascii_digit()) {
        return None;
    }
    let written = text
        .find(|character: char| !character.is_ascii_digit() && character != ',')
        .unwrap_or(text.len());
    let digits: String = text[..written]
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    Some((digits.parse().ok()?, written, &text[written..]))
}

#[cfg(test)]
mod tests;
