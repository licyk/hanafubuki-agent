use super::*;
use crate::{classify_remote_model_error, ModelError};

const GATING: [Option<u16>; 4] = [Some(400), Some(413), Some(422), None];

fn overflow(status: Option<u16>, code: Option<&str>, message: &str) -> bool {
    matches!(
        classify_remote_model_error(status, code.map(str::to_string), message.into(), None),
        ModelError::ContextWindowExceeded { .. }
    )
}

/// The Providers' own wordings, each of them in a refusal or in the stream.
#[test]
fn every_wording_counts_in_a_refusal_or_in_the_stream() {
    let messages = [
        "This model's maximum context length is 8192 tokens. However, your messages resulted in 9000 tokens.",
        "prompt is too long: 210000 tokens > 200000 maximum",
        "The input token count (1234567) exceeds the maximum number of tokens allowed (1048576).",
        "Input is too long for requested model.",
        "input length and `max_tokens` exceed context limit: 188240 + 21333 > 200000",
        "the request exceeds the context limit",
        "Request exceeded the context limit of the model",
        "Your input exceeds the context window of this model.",
        "Context window exceeded",
        "context_length_exceeded",
        "CONTEXT_WINDOW_EXCEEDED",
        "This model's maximum prompt length is 131072 but the request contains 150000 tokens.",
        "maximum input length reached",
        "the input length exceeds the context length",
        "too many tokens in the request",
        "Too many input tokens",
        "Token limit exceeded for this model",
        "token_limit_exceeded",
        "请求的上下文长度超过模型限制",
        "超出上下文窗口",
        "输入超出最大长度",
        "超过最大长度限制",
        "输入超过了最大 token 数",
        "输入超出模型长度限制",
        "超出长度限制",
        "输入过长",
        "token 数量超过 32768",
        "Token数超出限制",
        "TOKEN数量超出",
    ];
    for message in messages {
        for status in GATING {
            assert!(overflow(status, None, message), "{status:?} {message}");
        }
    }
}

#[test]
fn a_code_counts_whatever_the_message_and_any_status_but_429() {
    for code in [
        "context_length_exceeded",
        "Context_Window_Exceeded",
        "prompt_too_long",
    ] {
        for status in [Some(400), Some(500), None] {
            assert!(overflow(status, Some(code), "request failed"), "{code}");
        }
        assert!(!overflow(Some(429), Some(code), "request failed"), "{code}");
    }
}

/// A wording in the code (OpenRouter's `token_limit_exceeded`) counts as
/// one in the message does, behind the same status gate.
#[test]
fn a_wording_in_the_code_counts_behind_the_gate() {
    for status in GATING {
        assert!(overflow(
            status,
            Some("token_limit_exceeded"),
            "request failed"
        ));
    }
    assert!(!overflow(
        Some(500),
        Some("token_limit_exceeded"),
        "request failed"
    ));
}

/// The status gate: only a refusal or an in-band error is read, and a 429
/// never counts.
#[test]
fn a_wording_with_another_status_does_not_count() {
    let message = "This model's maximum context length is 8192 tokens.";
    for status in [Some(401), Some(404), Some(429), Some(500), Some(503)] {
        assert!(!overflow(status, None, message), "{status:?}");
    }
}

/// A rate limit or quota error never counts, though it speaks of tokens or
/// of a request too large, even in the stream, where no status gates it.
#[test]
fn a_rate_limit_or_quota_never_counts() {
    let tpm = "Request too large for gpt-4o in organization org-x on tokens per min (TPM): Limit 30000, Requested 45000. The input or output tokens must be reduced in order to run successfully.";
    for status in GATING {
        assert!(!overflow(status, Some("rate_limit_exceeded"), tpm));
        assert!(!overflow(status, None, tpm));
        assert!(!overflow(
            status,
            Some("insufficient_quota"),
            "too many tokens: you exceeded your current quota"
        ));
        assert!(!overflow(
            status,
            None,
            "Rate limit reached: too many tokens per minute"
        ));
        assert!(!overflow(status, None, "额度不足，上下文长度无法处理"));
        assert!(!overflow(status, None, "请求限流：输入过长"));
    }
}

/// What is left out as too broad, and unrelated refusals.
#[test]
fn unrelated_refusals_do_not_count() {
    for message in [
        "Request too large",
        "Request Entity Too Large",
        "Request exceeds the maximum allowed number of bytes.",
        "context length must be a positive integer",
        "invalid context window setting",
        "max_tokens must be less than or equal to 4096",
        "The image data is invalid",
        "",
    ] {
        for status in GATING {
            assert!(!overflow(status, None, message), "{status:?} {message}");
        }
    }
}

#[test]
fn the_limit_is_read_from_each_pattern() {
    for (message, limit) in [
        ("This model's maximum context length is 131072 tokens.", 131_072),
        ("maximum context length is 131,072 tokens", 131_072),
        ("maximum prompt length is 131072 but the request contains 150000 tokens", 131_072),
        ("Your input exceeds the context window of 200000 tokens", 200_000),
        ("the context length is 32768", 32_768),
        ("over the context limit of 8,192", 8_192),
        ("prompt is too long: 210000 tokens > 200000 maximum", 200_000),
        ("prompt is too long: 210,000 tokens >200,000 maximum", 200_000),
        (
            "The input token count (1234567) exceeds the maximum number of tokens allowed (1048576).",
            1_048_576,
        ),
        ("maximum input tokens: 64000", 64_000),
        ("maximum input token 64000", 64_000),
        ("the maximum allowed tokens is 4096", 4_096),
        ("exceeded the token limit of 65536 tokens", 65_536),
        ("token limit: 65,536 tok", 65_536),
        ("输入超过最大长度限制 32768 tokens", 32_768),
        ("最大上下文长度为 128000 tokens", 128_000),
    ] {
        assert_eq!(context_limit_in(message), Some(limit), "{message}");
    }
}

#[test]
fn a_limit_is_taken_only_within_its_bounds() {
    assert_eq!(
        context_limit_in("maximum context length is 1024 tokens"),
        Some(1_024)
    );
    assert_eq!(
        context_limit_in("maximum context length is 20,000,000 tokens"),
        Some(20_000_000)
    );
    assert_eq!(
        context_limit_in("maximum context length is 1023 tokens"),
        None
    );
    assert_eq!(
        context_limit_in("maximum context length is 20000001 tokens"),
        None
    );
    assert_eq!(
        context_limit_in("maximum context length is 99999999999999999999999 tokens"),
        None
    );
    // A value out of bounds moves on to the next pattern.
    assert_eq!(
        context_limit_in("maximum context length is 512 tokens; the context window of 4096"),
        Some(4_096)
    );
}

#[test]
fn a_refusal_that_names_no_limit_has_none() {
    for message in [
        "Your input exceeds the context window of this model.",
        "input length and `max_tokens` exceed context limit: 188240 + 21333 > 200000, decrease input length",
        "prompt is too long",
        "limit 512 tokens",
        "limit reached after a long wait of 9999 tokens",
        "输入过长",
        "",
    ] {
        assert_eq!(context_limit_in(message), None, "{message}");
    }
}
