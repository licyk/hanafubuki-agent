use crate::{classify_remote_model_error, ModelError};

fn classify(status: Option<u16>, code: Option<&str>, message: &str) -> ModelError {
    classify_remote_model_error(
        status,
        code.map(str::to_owned),
        message.to_owned(),
        Some("req-1".into()),
    )
}

fn is_structured_output_refusal(status: Option<u16>, code: Option<&str>, message: &str) -> bool {
    matches!(
        classify(status, code, message),
        ModelError::UnsupportedStructuredOutput { .. }
    )
}

fn is_tool_choice_refusal(status: Option<u16>, code: Option<&str>, message: &str) -> bool {
    matches!(
        classify(status, code, message),
        ModelError::UnsupportedToolChoice { .. }
    )
}

#[test]
fn refusals_of_native_structured_output_count_at_400_and_422() {
    let refusals = [
        // The reported refusal of a Responses-compatible gateway.
        (
            Some("invalid_input"),
            "Structured output (`text.format`) is not supported by this stateless gateway.",
        ),
        // Illustrative wordings of compatible servers, not recorded ones.
        (None, "Unknown parameter: 'text.format'."),
        (None, "response_format is not supported by this model"),
        (
            Some("unsupported_parameter"),
            "Unsupported parameter: 'response_format'",
        ),
        (None, "This endpoint does not support json_schema outputs"),
        (None, "structured_output: Extra inputs are not permitted"),
    ];
    for (code, message) in refusals {
        for status in [400, 422] {
            assert!(
                is_structured_output_refusal(Some(status), code, message),
                "{status} {code:?} {message}"
            );
        }
    }
}

#[test]
fn refusals_of_a_forced_tool_count_at_400_and_422() {
    let refusals = [
        // Illustrative wordings, not recorded ones.
        (
            Some("invalid_input"),
            "Forced tool calls (`tool_choice`) are not supported by this stateless gateway.",
        ),
        (None, "Unknown parameter: 'tool_choice'."),
        (None, "tools are not supported for this model"),
        (Some("unsupported_value"), "Invalid value for 'tool_choice'"),
    ];
    for (code, message) in refusals {
        for status in [400, 422] {
            assert!(
                is_tool_choice_refusal(Some(status), code, message),
                "{status} {code:?} {message}"
            );
        }
    }
}

#[test]
fn near_misses_are_not_refusals_of_a_rung() {
    // A schema the Provider found invalid names the parameter without
    // refusing it: the rung works, the request was wrong.
    let invalid_schema = "Invalid schema for response_format 'title': In context=(), 'required' is required to be supplied";
    assert!(matches!(
        classify(Some(400), Some("invalid_request_error"), invalid_schema),
        ModelError::Remote { .. }
    ));
    assert!(matches!(
        classify(Some(400), None, "Invalid value for 'text.format.schema'."),
        ModelError::Remote { .. }
    ));
    // A server error, or no status (an in-band error), never counts.
    let reported = "Structured output (`text.format`) is not supported by this stateless gateway.";
    assert!(matches!(
        classify(Some(500), None, reported),
        ModelError::Remote { .. }
    ));
    assert!(matches!(
        classify(None, None, reported),
        ModelError::Remote { .. }
    ));
    assert!(matches!(
        classify(Some(503), None, "tool_choice is not supported"),
        ModelError::Remote { .. }
    ));
}

#[test]
fn a_structured_output_refusal_wins_over_an_image_refusal() {
    // The image classifier takes any 400 that says it only supports text.
    assert!(is_structured_output_refusal(
        Some(400),
        None,
        "This gateway only supports text output; response_format is not supported."
    ));
    assert!(matches!(
        classify(Some(400), None, "This model only supports text input."),
        ModelError::UnsupportedImageInput { .. }
    ));
}

#[test]
fn a_refusal_naming_tools_keeps_a_turns_reasoning_and_image_fallbacks() {
    assert!(matches!(
        classify(
            Some(400),
            None,
            "reasoning_effort is not supported with tools for this model"
        ),
        ModelError::UnsupportedReasoning { .. }
    ));
    assert!(matches!(
        classify(
            Some(400),
            None,
            "image input is not supported when tools are present"
        ),
        ModelError::UnsupportedImageInput { .. }
    ));
}
