use serde_json::json;

use super::{function_arguments, token_usage};
use crate::{ModelUsage, MAX_TOOL_ARGUMENT_PREVIEW_BYTES};

#[test]
fn empty_arguments_are_an_empty_object_without_an_error() {
    for raw in ["", "  \n"] {
        assert_eq!(function_arguments("test", raw), (json!({}), None));
    }
}

#[test]
fn valid_arguments_are_kept_as_parsed() {
    assert_eq!(
        function_arguments("test", r#"{"scope":"all"}"#),
        (json!({"scope": "all"}), None)
    );
}

#[test]
fn unparseable_arguments_become_an_empty_object_with_a_bounded_preview() {
    let raw = format!("{{\"text\":\"{}", "好".repeat(300));
    let (arguments, error) = function_arguments("test", &raw);
    let error = error.expect("unparseable arguments carry an error");
    assert_eq!(arguments, json!({}));
    assert!(error.reason.contains("EOF"), "{}", error.reason);
    assert!(error.preview.len() <= MAX_TOOL_ARGUMENT_PREVIEW_BYTES);
    assert!(error.preview.len() > MAX_TOOL_ARGUMENT_PREVIEW_BYTES - "好".len());
    assert!(raw.starts_with(&error.preview));
}

#[test]
fn usage_with_a_missing_count_keeps_the_other_as_given() {
    let usage = json!({"input_tokens": 7, "output_tokens": null});
    assert_eq!(
        token_usage("test", Some(&usage), "input_tokens", "output_tokens"),
        Some(ModelUsage {
            input_tokens: 7,
            output_tokens: 0,
        })
    );
    let usage = json!({"output_tokens": 3});
    assert_eq!(
        token_usage("test", Some(&usage), "input_tokens", "output_tokens"),
        Some(ModelUsage {
            input_tokens: 0,
            output_tokens: 3,
        })
    );
}

#[test]
fn unusable_usage_is_no_usage() {
    for usage in [
        json!(null),
        json!([]),
        json!("7"),
        json!({}),
        json!({"input_tokens": -1, "output_tokens": 3}),
        json!({"input_tokens": "7", "output_tokens": 3}),
    ] {
        assert_eq!(
            token_usage("test", Some(&usage), "input_tokens", "output_tokens"),
            None,
            "{usage}"
        );
    }
    assert_eq!(
        token_usage("test", None, "input_tokens", "output_tokens"),
        None
    );
}
