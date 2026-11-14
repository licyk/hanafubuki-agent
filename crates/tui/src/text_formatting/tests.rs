// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;

#[test]
fn test_center_truncate_doesnt_truncate_short_path() {
    let sep = std::path::MAIN_SEPARATOR;
    let path = format!("{sep}Users{sep}codex{sep}Public");
    let truncated = center_truncate_path(&path, 40);

    assert_eq!(truncated, path);
}

#[test]
fn test_center_truncate_truncates_long_path() {
    let sep = std::path::MAIN_SEPARATOR;
    let path = format!("~{sep}hello{sep}the{sep}fox{sep}is{sep}very{sep}fast");
    let truncated = center_truncate_path(&path, 24);

    assert_eq!(
        truncated,
        format!("~{sep}hello{sep}the{sep}…{sep}very{sep}fast")
    );
}

#[test]
fn test_center_truncate_truncates_long_windows_path() {
    let sep = std::path::MAIN_SEPARATOR;
    let path = format!(
        "C:{sep}Users{sep}codex{sep}Projects{sep}super{sep}long{sep}windows{sep}path{sep}file.txt"
    );
    let truncated = center_truncate_path(&path, 36);

    let expected = format!("C:{sep}Users{sep}codex{sep}…{sep}path{sep}file.txt");

    assert_eq!(truncated, expected);
}

#[test]
fn test_center_truncate_handles_long_segment() {
    let sep = std::path::MAIN_SEPARATOR;
    let path = format!("~{sep}supercalifragilisticexpialidocious");
    let truncated = center_truncate_path(&path, 18);

    assert_eq!(truncated, format!("~{sep}…cexpialidocious"));
}

#[test]
fn strip_terminal_controls_drops_escapes_and_control_characters() {
    assert_eq!(
        strip_terminal_controls("\u{1b}[31mred\u{1b}[0m\r\nnext\tline\u{7}"),
        "red\nnext\tline"
    );
    assert_eq!(
        strip_terminal_controls("\u{1b}]0;title\u{7}text \u{1b}]8;;url\u{1b}\\link"),
        "text link"
    );
}

#[test]
fn test_truncate_text() {
    let text = "Hello, world!";
    let truncated = truncate_text(text, 8);
    assert_eq!(truncated, "Hello...");
}

#[test]
fn test_truncate_empty_string() {
    let text = "";
    let truncated = truncate_text(text, 5);
    assert_eq!(truncated, "");
}

#[test]
fn test_truncate_max_graphemes_zero() {
    let text = "Hello";
    let truncated = truncate_text(text, 0);
    assert_eq!(truncated, "");
}

#[test]
fn test_truncate_max_graphemes_one() {
    let text = "Hello";
    let truncated = truncate_text(text, 1);
    assert_eq!(truncated, "H");
}

#[test]
fn test_truncate_max_graphemes_two() {
    let text = "Hello";
    let truncated = truncate_text(text, 2);
    assert_eq!(truncated, "He");
}

#[test]
fn test_truncate_max_graphemes_three_boundary() {
    let text = "Hello";
    let truncated = truncate_text(text, 3);
    assert_eq!(truncated, "...");
}

#[test]
fn test_truncate_text_shorter_than_limit() {
    let text = "Hi";
    let truncated = truncate_text(text, 10);
    assert_eq!(truncated, "Hi");
}

#[test]
fn test_truncate_text_exact_length() {
    let text = "Hello";
    let truncated = truncate_text(text, 5);
    assert_eq!(truncated, "Hello");
}

#[test]
fn test_truncate_emoji() {
    let text = "👋🌍🚀✨💫";
    let truncated = truncate_text(text, 3);
    assert_eq!(truncated, "...");

    let truncated_longer = truncate_text(text, 4);
    assert_eq!(truncated_longer, "👋...");
}

// Codex's `format_json_compact` tests (57a808a392), unchanged.

#[test]
fn test_format_json_compact_simple_object() {
    let json = r#"{ "name": "John", "age": 30 }"#;
    let result = format_json_compact(json).unwrap();
    assert_eq!(result, r#"{"name": "John", "age": 30}"#);
}

#[test]
fn test_format_json_compact_nested_object() {
    let json = r#"{ "user": { "name": "John", "details": { "age": 30, "city": "NYC" } } }"#;
    let result = format_json_compact(json).unwrap();
    assert_eq!(
        result,
        r#"{"user": {"name": "John", "details": {"age": 30, "city": "NYC"}}}"#
    );
}

#[test]
fn test_format_json_compact_array() {
    let json = r#"[ 1, 2, { "key": "value" }, "string" ]"#;
    let result = format_json_compact(json).unwrap();
    assert_eq!(result, r#"[1, 2, {"key": "value"}, "string"]"#);
}

#[test]
fn test_format_json_compact_already_compact() {
    let json = r#"{"compact":true}"#;
    let result = format_json_compact(json).unwrap();
    assert_eq!(result, r#"{"compact": true}"#);
}

#[test]
fn test_format_json_compact_with_whitespace() {
    let json = r#"
        {
            "name": "John",
            "hobbies": [
                "reading",
                "coding"
            ]
        }
        "#;
    let result = format_json_compact(json).unwrap();
    assert_eq!(
        result,
        r#"{"name": "John", "hobbies": ["reading", "coding"]}"#
    );
}

#[test]
fn test_format_json_compact_invalid_json() {
    let invalid_json = r#"{"invalid": json syntax}"#;
    let result = format_json_compact(invalid_json);
    assert!(result.is_none());
}

#[test]
fn test_format_json_compact_empty_object() {
    let json = r#"{}"#;
    let result = format_json_compact(json).unwrap();
    assert_eq!(result, "{}");
}

#[test]
fn test_format_json_compact_empty_array() {
    let json = r#"[]"#;
    let result = format_json_compact(json).unwrap();
    assert_eq!(result, "[]");
}

#[test]
fn test_format_json_compact_primitive_values() {
    assert_eq!(format_json_compact("42").unwrap(), "42");
    assert_eq!(format_json_compact("true").unwrap(), "true");
    assert_eq!(format_json_compact("false").unwrap(), "false");
    assert_eq!(format_json_compact("null").unwrap(), "null");
    assert_eq!(format_json_compact(r#""string""#).unwrap(), r#""string""#);
}

// The reformatter's own behaviour.

#[test]
fn keys_keep_their_order_and_strings_their_spelling() {
    // Codex re-prints a parsed value; here the text is re-spaced, so a
    // string's separators, escapes and numbers stay as written.
    let json = r#"{"z":1,"a":{"b,c":"d: e","q":"\"quoted\", \\ é"},"n":1E5,"e":[ ],"o":{ }}"#;
    assert_eq!(
        format_json_compact(json).unwrap(),
        r#"{"z": 1, "a": {"b,c": "d: e", "q": "\"quoted\", \\ é"}, "n": 1E5, "e": [], "o": {}}"#
    );
    assert_eq!(
        format_json_pretty(json).unwrap(),
        [
            "{",
            r#"  "z": 1,"#,
            r#"  "a": {"#,
            r#"    "b,c": "d: e","#,
            r#"    "q": "\"quoted\", \\ é""#,
            "  },",
            r#"  "n": 1E5,"#,
            r#"  "e": [],"#,
            r#"  "o": {}"#,
            "}",
        ]
        .join("\n")
    );
}

#[test]
fn pretty_json_matches_serde_json_on_ordered_input() {
    // With keys already in order, the result is what serde_json prints.
    let json = r#"{"a":[1,2,{"b":null}],"c":{"d":true,"e":"中文"},"f":[[]]}"#;
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(
        format_json_pretty(json).unwrap(),
        serde_json::to_string_pretty(&value).unwrap()
    );
}

#[test]
fn what_is_not_one_json_value_is_left_as_it_is() {
    for text in [
        "",
        "{",
        r#"{"a": 1} trailing"#,
        r#"{"a": 1}{"b": 2}"#,
        "[1, 2,]",
        "not json",
        "{'single': 'quotes'}",
        "{\"a\": \"raw\u{1b}[31m control\"}",
    ] {
        assert_eq!(format_json_compact(text), None, "{text:?}");
        assert_eq!(format_json_pretty(text), None, "{text:?}");
    }
}

#[test]
fn hostile_json_is_bounded() {
    // Nesting past serde_json's depth: not laid out, and no stack used.
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert_eq!(format_json_pretty(&deep), None);
    assert_eq!(format_json_compact(&deep), None);
    let at_limit = format!("{}1{}", "[".repeat(128), "]".repeat(128));
    assert!(format_json_pretty(&at_limit).is_some());
    // As deep as the size bound allows: serde_json validates it without
    // recursing, and it is not laid out.
    let half = MAX_JSON_FORMAT_BYTES / 2;
    let deepest = format!("{}{}", "[".repeat(half), "]".repeat(half));
    assert_eq!(deepest.len(), MAX_JSON_FORMAT_BYTES);
    assert_eq!(format_json_compact(&deepest), None);
    // Over the size bound: shown as it came.
    let large = format!("[{}1]", "1,".repeat(MAX_JSON_FORMAT_BYTES / 2));
    assert_eq!(format_json_compact(&large), None);
    // Deep and wide: the indentation would multiply it.
    let inner = format!("[{}1]", "1,".repeat(30_000));
    let wide = format!("{}{inner}{}", "[".repeat(100), "]".repeat(100));
    assert!(format_json_compact(&wide).is_some());
    assert_eq!(format_json_pretty(&wide), None);
}
