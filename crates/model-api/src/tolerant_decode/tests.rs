use serde_json::json;

use super::{function_arguments, token_usage, UsageFields};
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
        token_usage("test", Some(&usage), &FIELDS),
        Some(ModelUsage {
            input_tokens: 7,
            output_tokens: 0,
            ..Default::default()
        })
    );
    let usage = json!({"output_tokens": 3});
    assert_eq!(
        token_usage("test", Some(&usage), &FIELDS),
        Some(ModelUsage {
            input_tokens: 0,
            output_tokens: 3,
            ..Default::default()
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
        assert_eq!(token_usage("test", Some(&usage), &FIELDS), None, "{usage}");
    }
    assert_eq!(token_usage("test", None, &FIELDS), None);
}

const FIELDS: UsageFields = UsageFields {
    input: "input_tokens",
    output: "output_tokens",
    cache_read: &["/details/cached_tokens", "/cache_hit_tokens"],
};

#[test]
fn the_first_cache_read_count_that_decodes_is_taken_and_never_added_to_the_input() {
    for (usage, cached) in [
        (
            json!({"details": {"cached_tokens": 4}, "cache_hit_tokens": 5}),
            Some(4),
        ),
        (json!({"details": null, "cache_hit_tokens": 5}), Some(5)),
        (
            json!({"details": {"cached_tokens": "4"}, "cache_hit_tokens": 5}),
            Some(5),
        ),
        (json!({"details": {}}), None),
    ] {
        let mut usage = usage;
        usage["input_tokens"] = json!(10);
        usage["output_tokens"] = json!(2);
        assert_eq!(
            token_usage("test", Some(&usage), &FIELDS),
            Some(ModelUsage {
                input_tokens: 10,
                output_tokens: 2,
                cache_read_tokens: cached,
                cache_write_tokens: None,
            }),
            "{usage}"
        );
    }
}

#[test]
fn a_malformed_cache_count_is_not_reported_and_keeps_the_counts() {
    let usage = json!({
        "input_tokens": 10,
        "output_tokens": 2,
        "details": {"cached_tokens": -1},
        "cache_hit_tokens": [3]
    });
    assert_eq!(
        token_usage("test", Some(&usage), &FIELDS),
        Some(ModelUsage {
            input_tokens: 10,
            output_tokens: 2,
            ..Default::default()
        })
    );
}
