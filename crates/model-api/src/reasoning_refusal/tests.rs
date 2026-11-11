use crate::{classify_remote_model_error, ModelError};

fn classify(status: Option<u16>, code: Option<&str>, message: &str) -> ModelError {
    classify_remote_model_error(
        status,
        code.map(str::to_owned),
        message.to_owned(),
        Some("req-1".into()),
    )
}

fn is_reasoning_refusal(status: Option<u16>, code: Option<&str>, message: &str) -> bool {
    matches!(
        classify(status, code, message),
        ModelError::UnsupportedReasoning { .. }
    )
}

#[test]
fn refusals_of_each_reasoning_parameter_count_at_400_and_422() {
    let refusals = [
        // OpenAI Responses, as reported in openai/openai-python#2532.
        (
            Some("invalid_request_error"),
            "Unsupported parameter: 'reasoning.effort' is not supported with this model.",
        ),
        // OpenAI Chat Completions, as reported on OpenAI's forum; the
        // `unsupported_value` wording is illustrative.
        (
            Some("unsupported_parameter"),
            "Unsupported parameter: 'reasoning_effort' is not supported with this model.",
        ),
        (
            Some("unsupported_value"),
            "Unsupported value: 'reasoning_effort' does not support 'minimal' with this model.",
        ),
        // Anthropic's documented refusals of a thinking form, then its
        // wording for an unknown field, applied to ours (illustrative).
        (
            Some("invalid_request_error"),
            "adaptive thinking is not supported on this model",
        ),
        (
            Some("invalid_request_error"),
            "\"thinking.type.enabled\" is not supported for this model. Use \"thinking.type.adaptive\" and \"output_config.effort\" to control thinking behavior.",
        ),
        (None, "output_config: Extra inputs are not permitted"),
        (None, "thinking: Extra inputs are not permitted"),
        // Illustrative wordings of compatible servers, not recorded ones.
        (None, "Unrecognized request argument supplied: reasoning_effort"),
        (None, "Unknown parameter: 'reasoning'."),
        (None, "create() got an unexpected keyword argument 'reasoning_effort'"),
        (None, "The parameter reasoning_effort is not allowed for this model"),
        (None, "this model doesn't support thinking"),
    ];
    for (code, message) in refusals {
        for status in [400, 422] {
            assert!(
                is_reasoning_refusal(Some(status), code, message),
                "{status} {code:?} {message}"
            );
        }
    }
}

#[test]
fn a_refusal_keeps_what_the_provider_said() {
    assert_eq!(
        classify(
            Some(400),
            Some("unsupported_parameter"),
            "Unsupported parameter: 'reasoning_effort'"
        ),
        ModelError::UnsupportedReasoning {
            status: Some(400),
            code: Some("unsupported_parameter".into()),
            message: "Unsupported parameter: 'reasoning_effort'".into(),
            request_id: Some("req-1".into()),
        }
    );
}

#[test]
fn a_refusal_code_counts_only_with_a_reasoning_parameter_named() {
    assert!(is_reasoning_refusal(
        Some(400),
        Some("unsupported_value"),
        "'reasoning_effort' must be one of 'low', 'medium' or 'high'"
    ));
    assert!(!is_reasoning_refusal(
        Some(400),
        Some("unsupported_parameter"),
        "Unsupported parameter: 'temperature' is not supported with this model."
    ));
}

#[test]
fn other_statuses_and_in_band_errors_never_count() {
    let message = "Unsupported parameter: 'reasoning_effort' is not supported with this model.";
    for status in [None, Some(401), Some(404), Some(413), Some(429), Some(500)] {
        assert!(
            !is_reasoning_refusal(status, Some("unsupported_parameter"), message),
            "{status:?}"
        );
    }
}

#[test]
fn unrelated_refusals_stay_remote() {
    let unrelated = [
        "Unsupported parameter: 'max_tokens' is not supported with this model.",
        "tool_choice: type \"tool\" and \"any\" are not supported for this model.",
        // Name a reasoning parameter, but refuse something else.
        "Thinking may not be enabled when tool_choice forces tool use.",
        "`max_tokens` must be greater than `thinking.budget_tokens`.",
        "thinking.budget_tokens: Input should be greater than or equal to 1024",
        "messages.1.content.0: Invalid `signature` in `thinking` block.",
        "Expected `thinking` or `redacted_thinking`, but found `tool_use`.",
    ];
    for message in unrelated {
        assert!(
            matches!(
                classify(Some(400), Some("invalid_request_error"), message),
                ModelError::Remote { .. }
            ),
            "{message}"
        );
    }
}

#[test]
fn context_overflows_and_image_refusals_stay_theirs() {
    assert!(matches!(
        classify(
            Some(400),
            Some("context_length_exceeded"),
            "This model's maximum context length is 8192 tokens, including reasoning; an unsupported amount."
        ),
        ModelError::ContextWindowExceeded { .. }
    ));
    assert!(matches!(
        classify(
            Some(400),
            None,
            "Image input is not supported by this model, with or without thinking."
        ),
        ModelError::UnsupportedImageInput { .. }
    ));
    assert!(matches!(
        classify(
            Some(429),
            Some("rate_limit_exceeded"),
            "Rate limit reached for reasoning tokens; unsupported burst."
        ),
        ModelError::Remote { .. }
    ));
}
