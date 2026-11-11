use serde_json::json;

use super::*;

#[test]
fn a_window_is_the_first_field_with_a_positive_integer() {
    let cases = [
        (
            json!({"max_input_tokens": 200_000, "context_length": 1}),
            Some(200_000),
        ),
        (
            json!({"max_input_tokens": 0, "context_length": 131_072}),
            Some(131_072),
        ),
        (json!({"max_model_len": 32_768}), Some(32_768)),
        (
            json!({"context_window": "large", "contextWindow": 65_536}),
            Some(65_536),
        ),
        (
            json!({"top_provider": {"context_length": 8192}}),
            Some(8192),
        ),
        (
            json!({"context_length": 4096, "top_provider": {"context_length": 8192}}),
            Some(4096),
        ),
        (json!({"context_length": -1, "context_window": 1.5e5}), None),
        (json!({"context_length": null, "top_provider": 8192}), None),
        (json!({}), None),
    ];
    for (item, expected) in cases {
        assert_eq!(context_window(&item), expected, "{item}");
    }
}

#[test]
fn max_output_takes_the_documented_fields_in_order() {
    let cases = [
        (
            json!({"max_output_tokens": 64_000, "max_tokens": 1}),
            Some(64_000),
        ),
        (json!({"max_completion_tokens": 32_768}), Some(32_768)),
        (json!({"maxTokens": 16_384}), Some(16_384)),
        (
            json!({"max_tokens": 128_000, "top_provider": {"max_completion_tokens": 4096}}),
            Some(4096),
        ),
        (json!({"max_tokens": 128_000}), Some(128_000)),
        (json!({"max_tokens": 0, "maxTokens": true}), None),
    ];
    for (item, expected) in cases {
        assert_eq!(max_output_tokens(&item), expected, "{item}");
    }
}

#[test]
fn image_input_is_yes_when_any_field_says_yes() {
    let cases = [
        (
            json!({"architecture": {"input_modalities": ["text", "image"]}}),
            Some(true),
        ),
        (json!({"input_modalities": ["TEXT", "Image"]}), Some(true)),
        (json!({"input": ["text", "image"]}), Some(true)),
        (
            json!({"architecture": {"input_modalities": ["text"]}}),
            Some(false),
        ),
        (json!({"vision": true}), Some(true)),
        (json!({"capabilities": {"vision": false}}), Some(false)),
        (
            json!({"capabilities": {"image_input": {"supported": true}}}),
            Some(true),
        ),
        (
            json!({"capabilities": {"image_input": {"supported": false}}}),
            Some(false),
        ),
        // A boolean and a list that disagree: the yes wins whichever comes first.
        (
            json!({"vision": false, "input_modalities": ["image"]}),
            Some(true),
        ),
        (
            json!({"vision": true, "input_modalities": ["text"]}),
            Some(true),
        ),
        // Empty lists, lists of non-strings and non-booleans say nothing.
        (json!({"input_modalities": [], "input": [1, 2]}), None),
        (json!({"vision": "yes", "input": "image"}), None),
        (
            json!({"input_modalities": [], "vision": false}),
            Some(false),
        ),
        (json!({}), None),
    ];
    for (item, expected) in cases {
        assert_eq!(image_input(&item), expected, "{item}");
    }
}

#[test]
fn reasoning_is_yes_when_any_field_or_reported_level_says_yes() {
    let cases = [
        (
            json!({"supported_parameters": ["tools", "reasoning"]}),
            false,
            Some(true),
        ),
        (
            json!({"supported_parameters": ["reasoning_effort"]}),
            false,
            Some(true),
        ),
        (
            json!({"supported_parameters": ["temperature", "max_tokens"]}),
            false,
            Some(false),
        ),
        (json!({"reasoning": true}), false, Some(true)),
        (
            json!({"capabilities": {"reasoning": false}}),
            false,
            Some(false),
        ),
        (
            json!({"capabilities": {"thinking": {"supported": true}}}),
            false,
            Some(true),
        ),
        (
            json!({"supported_parameters": ["temperature"], "capabilities": {"thinking": {"supported": true}}}),
            false,
            Some(true),
        ),
        (
            json!({"supported_parameters": ["temperature"]}),
            true,
            Some(true),
        ),
        (json!({}), true, Some(true)),
        // OpenRouter's `reasoning` is an object of effort settings, not a flag.
        (json!({"reasoning": {"mandatory": false}}), false, None),
        (json!({"supported_parameters": []}), false, None),
        (json!({}), false, None),
    ];
    for (item, reports_levels, expected) in cases {
        assert_eq!(reasoning(&item, reports_levels), expected, "{item}");
    }
}
